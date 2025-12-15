// Variable management (对齐lparser.c的变量相关函数)
use super::*;
use super::expdesc::*;
use super::helpers;

/// Search for a local variable by name (对齐searchvar)
/// Returns the expression kind if found, -1 otherwise
pub(crate) fn searchvar(c: &mut Compiler, name: &str, var: &mut ExpDesc) -> i32 {
    // 关键修复：只搜索已激活的局部变量（nactvar个）
    // 参考lparser.c:406: for (i = cast_int(fs->nactvar) - 1; i >= 0; i--)
    let scope = c.scope_chain.borrow();
    for i in (0..c.nactvar).rev() {
        if i < scope.locals.len() && scope.locals[i].name == name {
            let local = &scope.locals[i];
            // Check if it's a compile-time constant
            if local.is_const {
                // VK: compile-time constant
                var.kind = ExpKind::VK;
                var.info = i as u32;
                return ExpKind::VK as i32;
            } else {
                // VLOCAL: regular local variable
                init_var(c, var, i);
                return ExpKind::VLocal as i32;
            }
        }
    }
    -1  // not found
}

/// Initialize a variable expression (对齐init_var)
fn init_var(c: &Compiler, var: &mut ExpDesc, vidx: usize) {
    var.kind = ExpKind::VLocal;
    var.var.vidx = vidx;
    
    // Get the register index for this variable
    let scope = c.scope_chain.borrow();
    if vidx < scope.locals.len() {
        var.var.ridx = scope.locals[vidx].reg;
    } else {
        var.var.ridx = 0;
    }
}

/// Create a new local variable (对齐new_localvar)
/// Returns the variable index
pub(crate) fn new_localvar(c: &mut Compiler, name: String) -> Result<usize, String> {
    // Check limit
    let mut scope = c.scope_chain.borrow_mut();
    if scope.locals.len() >= 200 {  // MAXVARS
        return Err("too many local variables (limit is 200)".to_string());
    }
    
    let local = Local {
        name,
        depth: c.scope_depth,
        reg: 0,  // Will be set by adjustlocalvars
        is_const: false,
        is_to_be_closed: false,
        needs_close: false,
    };
    
    scope.locals.push(local);
    Ok(scope.locals.len() - 1)
}

/// Get the register level for a variable index (对齐reglevel)
pub(crate) fn reglevel(c: &Compiler, nvar: usize) -> u32 {
    let scope = c.scope_chain.borrow();
    let mut i = nvar;
    while i > 0 {
        i -= 1;
        if i < scope.locals.len() {
            let local = &scope.locals[i];
            if !local.is_const {  // is in a register?
                return local.reg + 1;
            }
        }
    }
    0  // no variables in registers
}

/// Activate local variables (对齐adjustlocalvars)
/// Makes the last 'nvars' variables visible in the current scope
pub(crate) fn adjustlocalvars(c: &mut Compiler, nvars: usize) {
    let reglevel = helpers::nvarstack(c);
    let mut scope = c.scope_chain.borrow_mut();
    
    for i in 0..nvars {
        let vidx = c.nactvar;
        c.nactvar += 1;
        
        if vidx < scope.locals.len() {
            let local = &mut scope.locals[vidx];
            local.reg = reglevel + i as u32;
            // TODO: Register local variable for debug info
        }
    }
}

/// Mark that a variable will be used as an upvalue (对齐markupval)
fn markupval(c: &mut Compiler, level: usize) {
    // Mark local variable as needing close (对齐luac markupval)
    let mut scope = c.scope_chain.borrow_mut();
    if level < scope.locals.len() {
        scope.locals[level].needs_close = true;
    }
    drop(scope);
    
    // Find the block where this variable was defined and mark it
    let mut current = &mut c.block;
    
    loop {
        match current {
            Some(block) => {
                if block.nactvar <= level {
                    block.upval = true;
                    c.needclose = true;
                    break;
                }
                current = &mut block.previous;
            }
            None => break,
        }
    }
}

/// Search for an existing upvalue by name (对齐searchupvalue)
fn searchupvalue(c: &Compiler, name: &str) -> i32 {
    let scope = c.scope_chain.borrow();
    for (i, upval) in scope.upvalues.iter().enumerate() {
        if upval.name == name {
            return i as i32;
        }
    }
    -1  // not found
}

/// Create a new upvalue (对齐newupvalue)
fn newupvalue(c: &mut Compiler, name: String, var: &ExpDesc) -> Result<u32, String> {
    let mut scope = c.scope_chain.borrow_mut();
    
    if scope.upvalues.len() >= 255 {  // Max upvalues
        return Err("too many upvalues (limit is 255)".to_string());
    }
    
    let upvalue = Upvalue {
        name,
        is_local: matches!(var.kind, ExpKind::VLocal),
        index: var.info,
    };
    
    scope.upvalues.push(upvalue);
    Ok((scope.upvalues.len() - 1) as u32)
}

/// Find variable recursively through scopes (对齐singlevaraux)
fn singlevaraux(c: &mut Compiler, name: &str, var: &mut ExpDesc, base: bool) -> Result<(), String> {
    // Try to find as local variable in current scope
    let v = searchvar(c, name, var);
    
    if v >= 0 {
        // Found as local variable
        if v == ExpKind::VLocal as i32 && !base {
            // Mark that this local will be used as an upvalue
            markupval(c, var.var.vidx);
        }
        return Ok(());
    }
    
    // Not found locally, try existing upvalues first (对齐lparser.c:445)
    let mut idx = searchupvalue(c, name);
    
    if idx < 0 {
        // Not found as existing upvalue, check if we have a parent compiler
        if let Some(parent_ptr) = c.prev {
            // Try to find in parent scope (对齐luac: recursively search in parent)
            unsafe {
                let parent = &mut *parent_ptr;
                
                // Recursively search in parent (not base level anymore)
                singlevaraux(parent, name, var, false)?;
                
                // Check what we found in parent (对齐lparser.c:448-451)
                match var.kind {
                    ExpKind::VLocal | ExpKind::VUpval => {
                        // Found in parent as local or upvalue - create NEW upvalue in current function
                        // 对齐luac的newupvalue调用
                        idx = newupvalue(c, name.to_string(), var)? as i32;
                    }
                    ExpKind::VVoid => {
                        // Not found in parent either - will be treated as global
                        return Ok(());
                    }
                    _ => {
                        // Other kinds (constants etc) - don't need upvalue at this level
                        return Ok(());
                    }
                }
            }
        } else {
            // No parent and no existing upvalue - not found
            var.kind = ExpKind::VVoid;
            var.info = 0;
            return Ok(());
        }
    }
    
    // Found as upvalue (either existing or newly created)
    // 对齐lparser.c:453: init_exp(var, VUPVAL, idx);
    var.kind = ExpKind::VUpval;
    var.info = idx as u32;
    Ok(())
}

/// Find a variable (对齐singlevar)
/// Handles local variables, upvalues, and global variables
pub(crate) fn singlevar(c: &mut Compiler, name: &str, var: &mut ExpDesc) -> Result<(), String> {
    singlevaraux(c, name, var, true)?;
    
    if matches!(var.kind, ExpKind::VVoid) {
        // Not found as local or upvalue - treat as global variable
        // Global variable access: _ENV[name]
        // 参考lparser.c:467-473
        
        // First, get _ENV variable (could be local or upvalue)
        let mut env_var = ExpDesc::new_void();
        singlevaraux(c, "_ENV", &mut env_var, true)?;
        
        if matches!(env_var.kind, ExpKind::VVoid) {
            return Err(format!("Cannot access global variable '{}': _ENV not available", name));
        }
        
        // 参考lparser.c:471: luaK_exp2anyregup(fs, var)
        // Convert _ENV to register if it's not an upvalue or has jumps
        if env_var.kind != ExpKind::VUpval || exp2reg::has_jumps(&env_var) {
            exp2reg::exp2anyreg(c, &mut env_var);
        }
        
        // Create key expression for the variable name
        // 参考lparser.c:472: codestring(&key, varname)
        let name_idx = super::helpers::string_k(c, name.to_string());
        let mut key = ExpDesc::new_void();
        key.kind = ExpKind::VKStr;
        key.info = name_idx;
        
        // 参考lparser.c:473: luaK_indexed(fs, var, &key)
        // Use indexed function to create the proper indexed expression
        *var = env_var;
        exp2reg::indexed(c, var, &mut key);
    }
    
    Ok(())
}
