use crate::compiler::parse_lua_number::NumberResult;

// Expression compilation (对齐lparser.c的expression parsing)
use super::expdesc::*;
use super::helpers;
use super::var::*;
use super::*;
use emmylua_parser::*;

/// 编译表达式 (对齐 lparser.c 的 expr)
/// emmylua_parser 的 AST 已经处理了优先级，直接递归编译即可
pub(crate) fn expr(c: &mut Compiler, node: &LuaExpr) -> Result<ExpDesc, String> {
    match node {
        // 一元运算符
        LuaExpr::UnaryExpr(unary) => {
            let operand = unary.get_expr().ok_or("unary expression missing operand")?;
            let op_token = unary
                .get_op_token()
                .ok_or("unary expression missing operator")?;

            // 递归编译操作数
            let mut v = expr(c, &operand)?;

            // 应用一元运算符
            apply_unary_op(c, &op_token, &mut v)?;
            Ok(v)
        }

        // 二元运算符
        LuaExpr::BinaryExpr(binary) => {
            let (left, right) = binary
                .get_exprs()
                .ok_or("binary expression missing operands")?;
            let op_token = binary
                .get_op_token()
                .ok_or("binary expression missing operator")?;

            // 递归编译左操作数
            let mut v1 = expr(c, &left)?;

            // 中缀处理
            infix_op(c, &op_token, &mut v1)?;

            // 递归编译右操作数
            let mut v2 = expr(c, &right)?;

            // 后缀处理
            postfix_op(c, &op_token, &mut v1, &mut v2)?;

            Ok(v1)
        }

        // 其他表达式
        _ => simple_exp(c, node),
    }
}

/// Compile a simple expression (对齐simpleexp)
pub(crate) fn simple_exp(c: &mut Compiler, node: &LuaExpr) -> Result<ExpDesc, String> {
    use super::helpers;

    match node {
        LuaExpr::LiteralExpr(lit) => {
            // Try to get the text and parse it
            match lit.get_literal().unwrap() {
                LuaLiteralToken::Bool(b) => {
                    if b.is_true() {
                        Ok(ExpDesc::new_true())
                    } else {
                        Ok(ExpDesc::new_false())
                    }
                }
                LuaLiteralToken::Nil(_) => Ok(ExpDesc::new_nil()),
                LuaLiteralToken::Number(n) => {
                    if n.is_int() {
                        match parse_lua_number::int_token_value(n.syntax()) {
                            Ok(NumberResult::Int(i)) => Ok(ExpDesc::new_int(i)),
                            Ok(NumberResult::Uint(u)) => {
                                if u <= i64::MAX as u64 {
                                    Ok(ExpDesc::new_int(u as i64))
                                } else {
                                    Err(format!(
                                        "The integer literal '{}' is too large to be represented as a signed integer",
                                        n.syntax().text()
                                    ))
                                }
                            }
                            Ok(NumberResult::Float(f)) => Ok(ExpDesc::new_float(f)),
                            Err(e) => Err(e),
                        }
                    } else {
                        Ok(ExpDesc::new_float(n.get_float_value()))
                    }
                }
                LuaLiteralToken::String(s) => {
                    let str_val = s.get_value();
                    let k = helpers::string_k(c, str_val.to_string());
                    Ok(ExpDesc::new_k(k))
                }
                LuaLiteralToken::Dots(_) => {
                    // Vararg expression (对齐lparser.c中的TK_DOTS处理)
                    // 检查当前函数是否为vararg
                    if !c.chunk.is_vararg {
                        return Err("cannot use '...' outside a vararg function".to_string());
                    }
                    // OP_VARARG A B : R[A], R[A+1], ..., R[A+B-2] = vararg
                    // B=1 表示返回所有可变参数
                    let pc = helpers::code_abc(c, OpCode::Vararg, 0, 1, 0);
                    Ok(ExpDesc {
                        kind: ExpKind::VVararg,
                        info: pc as u32,
                        ival: 0,
                        nval: 0.0,
                        ind: expdesc::IndexInfo { t: 0, idx: 0 },
                        var: expdesc::VarInfo { ridx: 0, vidx: 0 },
                        t: -1,
                        f: -1,
                    })
                }
                _ => Err("Unsupported literal type".to_string()),
            }
        }
        LuaExpr::NameExpr(name) => {
            // Variable reference (对齐singlevar)
            let name_text = name
                .get_name_token()
                .ok_or("Name expression missing token")?
                .get_name_text()
                .to_string();

            let mut v = ExpDesc::new_void();
            super::var::singlevar(c, &name_text, &mut v)?;
            Ok(v)
        }
        LuaExpr::IndexExpr(index_expr) => {
            // Table indexing: t[k] or t.k (对齐suffixedexp中的索引部分)
            compile_index_expr(c, index_expr)
        }
        LuaExpr::ParenExpr(paren) => {
            // Parenthesized expression
            if let Some(inner) = paren.get_expr() {
                let mut v = expr(c, &inner)?;
                // Discharge to ensure value is computed
                super::exp2reg::discharge_vars(c, &mut v);
                Ok(v)
            } else {
                Ok(ExpDesc::new_nil())
            }
        }
        LuaExpr::ClosureExpr(closure_expr) => {
            // Anonymous function / closure (对齐body)
            compile_closure_expr(c, closure_expr)
        }
        LuaExpr::CallExpr(call_expr) => {
            // Function call expression (对齐funcargs)
            compile_function_call(c, call_expr)
        }
        _ => {
            // TODO: Handle other expression types (calls, tables, binary ops, etc.)
            Err(format!("Unsupported expression type: {:?}", node))
        }
    }
}

/// Compile index expression: t[k] or t.field or t:method (对齐yindex和fieldsel)
pub(crate) fn compile_index_expr(
    c: &mut Compiler,
    index_expr: &LuaIndexExpr,
) -> Result<ExpDesc, String> {
    // Get the prefix expression (table)
    let prefix = index_expr
        .get_prefix_expr()
        .ok_or("Index expression missing prefix")?;

    let mut t = expr(c, &prefix)?;

    // Discharge table to register or upvalue
    super::exp2reg::exp2anyregup(c, &mut t);

    // Get the index/key
    if let Some(index_token) = index_expr.get_index_token() {
        // 冒号语法不应该出现在普通索引表达式中
        // 冒号只在函数调用（t:method()）和函数定义（function t:method()）中有意义
        if index_token.is_colon() {
            return Err(
                "colon syntax ':' is only valid in function calls or definitions".to_string(),
            );
        }

        if index_token.is_dot() {
            // Dot notation: t.field
            // Get the field name as a string constant
            if let Some(key) = index_expr.get_index_key() {
                let key_name = match key {
                    LuaIndexKey::Name(name_token) => name_token.get_name_text().to_string(),
                    _ => return Err("Dot notation requires name key".to_string()),
                };

                // Create string constant for field name
                let k_idx = helpers::string_k(c, key_name);
                let mut k = ExpDesc::new_k(k_idx);

                // Create indexed expression
                super::exp2reg::indexed(c, &mut t, &mut k);
                return Ok(t);
            }
        } else if index_token.is_left_bracket() {
            // Bracket notation: t[expr]
            if let Some(key) = index_expr.get_index_key() {
                let mut k = match key {
                    LuaIndexKey::Expr(key_expr) => expr(c, &key_expr)?,
                    LuaIndexKey::Name(name_token) => {
                        // In bracket context, treat name as variable reference
                        let name_text = name_token.get_name_text().to_string();
                        let mut v = ExpDesc::new_void();
                        super::var::singlevar(c, &name_text, &mut v)?;
                        v
                    }
                    LuaIndexKey::String(str_token) => {
                        // String literal key
                        let str_val = str_token.get_value();
                        let k_idx = helpers::string_k(c, str_val.to_string());
                        ExpDesc::new_k(k_idx)
                    }
                    LuaIndexKey::Integer(int_token) => {
                        // Integer literal key
                        ExpDesc::new_int(int_token.get_int_value())
                    }
                    LuaIndexKey::Idx(_) => {
                        // Generic index (shouldn't normally happen in well-formed code)
                        return Err("Invalid index key type".to_string());
                    }
                };

                // Ensure key value is computed
                super::exp2reg::exp2val(c, &mut k);

                // Create indexed expression
                super::exp2reg::indexed(c, &mut t, &mut k);
                return Ok(t);
            }
        }
    }

    Err("Invalid index expression".to_string())
}

/// 应用一元运算符 (对齐 luaK_prefix)
fn apply_unary_op(
    c: &mut Compiler,
    op_token: &LuaUnaryOpToken,
    v: &mut ExpDesc,
) -> Result<(), String> {
    use super::helpers;
    use OpCode;
    use emmylua_parser::UnaryOperator;

    let op = op_token.get_op();

    match op {
        UnaryOperator::OpUnm => {
            // 负号：尝试常量折叠
            if v.kind == ExpKind::VKInt {
                v.ival = v.ival.wrapping_neg();
            } else if v.kind == ExpKind::VKFlt {
                v.nval = -v.nval;
            } else {
                // 生成 UNM 指令
                super::exp2reg::discharge_2any_reg(c, v);
                super::exp2reg::free_exp(c, v);
                v.info = helpers::code_abc(c, OpCode::Unm, 0, v.info, 0) as u32;
                v.kind = ExpKind::VReloc;
            }
        }
        UnaryOperator::OpNot => {
            // 逻辑非：常量折叠或生成 NOT 指令
            if expdesc::is_const(v) {
                // 常量折叠
                let val = matches!(v.kind, ExpKind::VNil | ExpKind::VFalse);
                *v = if val {
                    ExpDesc::new_true()
                } else {
                    ExpDesc::new_false()
                };
            } else {
                super::exp2reg::discharge_2any_reg(c, v);
                super::exp2reg::free_exp(c, v);
                v.info = helpers::code_abc(c, OpCode::Not, 0, v.info, 0) as u32;
                v.kind = ExpKind::VReloc;
            }
        }
        UnaryOperator::OpLen => {
            // 长度运算符
            super::exp2reg::discharge_2any_reg(c, v);
            super::exp2reg::free_exp(c, v);
            v.info = helpers::code_abc(c, OpCode::Len, 0, v.info, 0) as u32;
            v.kind = ExpKind::VReloc;
        }
        UnaryOperator::OpBNot => {
            // 按位取反
            if v.kind == ExpKind::VKInt {
                v.ival = !v.ival;
            } else {
                super::exp2reg::discharge_2any_reg(c, v);
                super::exp2reg::free_exp(c, v);
                v.info = helpers::code_abc(c, OpCode::BNot, 0, v.info, 0) as u32;
                v.kind = ExpKind::VReloc;
            }
        }
        UnaryOperator::OpNop => {
            // 空操作，不应该出现
        }
    }

    Ok(())
}

/// 中缀处理 (对齐 luaK_infix)
fn infix_op(c: &mut Compiler, op_token: &LuaBinaryOpToken, v: &mut ExpDesc) -> Result<(), String> {
    use emmylua_parser::BinaryOperator;

    let op = op_token.get_op();

    match op {
        BinaryOperator::OpAnd => {
            // and: 短路求值，左操作数为 false 时跳过右操作数
            super::exp2reg::goiftrue(c, v);
        }
        BinaryOperator::OpOr => {
            // or: 短路求值，左操作数为 true 时跳过右操作数
            super::exp2reg::goiffalse(c, v);
        }
        BinaryOperator::OpConcat => {
            // 字符串连接：需要把左操作数放到寄存器
            super::exp2reg::exp2nextreg(c, v);
        }
        BinaryOperator::OpAdd
        | BinaryOperator::OpSub
        | BinaryOperator::OpMul
        | BinaryOperator::OpDiv
        | BinaryOperator::OpIDiv
        | BinaryOperator::OpMod
        | BinaryOperator::OpPow
        | BinaryOperator::OpBAnd
        | BinaryOperator::OpBOr
        | BinaryOperator::OpBXor
        | BinaryOperator::OpShl
        | BinaryOperator::OpShr => {
            // 算术和按位运算：常量折叠在 postfix 中处理
            // 如果左操作数不是数值常量，则放到寄存器
            if !expdesc::is_numeral(v) {
                super::exp2reg::exp2anyreg(c, v);
            }
        }
        BinaryOperator::OpEq
        | BinaryOperator::OpNe
        | BinaryOperator::OpLt
        | BinaryOperator::OpLe
        | BinaryOperator::OpGt
        | BinaryOperator::OpGe => {
            // 比较运算：不需要在 infix 阶段做特殊处理
        }
        BinaryOperator::OpNop => {}
    }

    Ok(())
}

/// 生成算术运算指令（对齐 luaK_codearith）
fn code_arith(
    c: &mut Compiler,
    op: OpCode,
    e1: &mut ExpDesc,
    e2: &mut ExpDesc,
) -> Result<(), String> {
    // 尝试常量折叠
    if try_const_folding(op, e1, e2) {
        return Ok(());
    }
    // 生成运算指令
    code_bin_arith(c, op, e1, e2);
    Ok(())
}

/// 常量折叠（对齐 constfolding）
fn try_const_folding(op: OpCode, e1: &mut ExpDesc, e2: &ExpDesc) -> bool {
    use OpCode;

    // 只对数值常量进行折叠
    if !expdesc::is_numeral(e1) || !expdesc::is_numeral(e2) {
        return false;
    }

    // 获取操作数值
    let v1 = if e1.kind == ExpKind::VKInt {
        e1.ival as f64
    } else {
        e1.nval
    };
    let v2 = if e2.kind == ExpKind::VKInt {
        e2.ival as f64
    } else {
        e2.nval
    };

    // 执行运算
    let result = match op {
        OpCode::Add => v1 + v2,
        OpCode::Sub => v1 - v2,
        OpCode::Mul => v1 * v2,
        OpCode::Div => v1 / v2,
        OpCode::IDiv => (v1 / v2).floor(),
        OpCode::Mod => v1 % v2,
        OpCode::Pow => v1.powf(v2),
        OpCode::BAnd if e1.kind == ExpKind::VKInt && e2.kind == ExpKind::VKInt => {
            e1.ival &= e2.ival;
            return true;
        }
        OpCode::BOr if e1.kind == ExpKind::VKInt && e2.kind == ExpKind::VKInt => {
            e1.ival |= e2.ival;
            return true;
        }
        OpCode::BXor if e1.kind == ExpKind::VKInt && e2.kind == ExpKind::VKInt => {
            e1.ival ^= e2.ival;
            return true;
        }
        OpCode::Shl if e1.kind == ExpKind::VKInt && e2.kind == ExpKind::VKInt => {
            e1.ival = e1.ival.wrapping_shl(e2.ival as u32);
            return true;
        }
        OpCode::Shr if e1.kind == ExpKind::VKInt && e2.kind == ExpKind::VKInt => {
            e1.ival = e1.ival.wrapping_shr(e2.ival as u32);
            return true;
        }
        _ => return false,
    };

    // 保存结果
    e1.nval = result;
    e1.kind = ExpKind::VKFlt;
    true
}

/// 生成二元算术指令（对齐 codebinarith）
fn code_bin_arith(c: &mut Compiler, op: OpCode, e1: &mut ExpDesc, e2: &mut ExpDesc) {
    use super::helpers;

    let o2 = super::exp2reg::exp2anyreg(c, e2);
    let o1 = super::exp2reg::exp2anyreg(c, e1);

    if o1 > o2 {
        super::exp2reg::free_exp(c, e1);
        super::exp2reg::free_exp(c, e2);
    } else {
        super::exp2reg::free_exp(c, e2);
        super::exp2reg::free_exp(c, e1);
    }

    e1.info = helpers::code_abc(c, op, 0, o1, o2) as u32;
    e1.kind = ExpKind::VReloc;
}

/// 生成比较指令（对齐 codecomp）
fn code_comp(c: &mut Compiler, op: OpCode, e1: &mut ExpDesc, e2: &mut ExpDesc) {
    use super::helpers;

    let o1 = super::exp2reg::exp2anyreg(c, e1);
    let o2 = super::exp2reg::exp2anyreg(c, e2);

    super::exp2reg::free_exp(c, e2);
    super::exp2reg::free_exp(c, e1);

    // 生成比较指令（结果是跳转）
    e1.info = helpers::cond_jump(c, op, o1, o2) as u32;
    e1.kind = ExpKind::VJmp;
}

/// 后缀处理 (对齐 luaK_posfix)
fn postfix_op(
    c: &mut Compiler,
    op_token: &LuaBinaryOpToken,
    v1: &mut ExpDesc,
    v2: &mut ExpDesc,
) -> Result<(), String> {
    use super::helpers;
    use OpCode;
    use emmylua_parser::BinaryOperator;

    let op = op_token.get_op();

    match op {
        BinaryOperator::OpAnd => {
            // and: v1 and v2
            debug_assert!(v1.t == helpers::NO_JUMP); // 左操作数为 true 时继续
            super::exp2reg::discharge_2any_reg(c, v2);
            helpers::concat(c, &mut v2.f, v1.f);
            *v1 = v2.clone();
        }
        BinaryOperator::OpOr => {
            // or: v1 or v2
            debug_assert!(v1.f == helpers::NO_JUMP); // 左操作数为 false 时继续
            super::exp2reg::discharge_2any_reg(c, v2);
            helpers::concat(c, &mut v2.t, v1.t);
            *v1 = v2.clone();
        }
        BinaryOperator::OpConcat => {
            // 字符串连接: v1 .. v2
            super::exp2reg::exp2val(c, v2);
            if v2.kind == ExpKind::VReloc && helpers::get_op(c, v2.info) == OpCode::Concat {
                // 连接链：v1 .. v2 .. v3 => CONCAT A B C
                debug_assert!(v1.info == helpers::getarg_b(c, v2.info) as u32 - 1);
                super::exp2reg::free_exp(c, v1);
                helpers::setarg_b(c, v2.info, v1.info);
                v1.kind = ExpKind::VReloc;
                v1.info = v2.info;
            } else {
                // 简单连接
                super::exp2reg::exp2nextreg(c, v2);
                code_bin_arith(c, OpCode::Concat, v1, v2);
            }
        }
        // 算术运算
        BinaryOperator::OpAdd => code_arith(c, OpCode::Add, v1, v2)?,
        BinaryOperator::OpSub => code_arith(c, OpCode::Sub, v1, v2)?,
        BinaryOperator::OpMul => code_arith(c, OpCode::Mul, v1, v2)?,
        BinaryOperator::OpDiv => code_arith(c, OpCode::Div, v1, v2)?,
        BinaryOperator::OpIDiv => code_arith(c, OpCode::IDiv, v1, v2)?,
        BinaryOperator::OpMod => code_arith(c, OpCode::Mod, v1, v2)?,
        BinaryOperator::OpPow => code_arith(c, OpCode::Pow, v1, v2)?,
        // 按位运算
        BinaryOperator::OpBAnd => code_arith(c, OpCode::BAnd, v1, v2)?,
        BinaryOperator::OpBOr => code_arith(c, OpCode::BOr, v1, v2)?,
        BinaryOperator::OpBXor => code_arith(c, OpCode::BXor, v1, v2)?,
        BinaryOperator::OpShl => code_arith(c, OpCode::Shl, v1, v2)?,
        BinaryOperator::OpShr => code_arith(c, OpCode::Shr, v1, v2)?,
        // 比较运算
        BinaryOperator::OpEq => code_comp(c, OpCode::Eq, v1, v2),
        BinaryOperator::OpNe => {
            code_comp(c, OpCode::Eq, v1, v2);
            // ~= 是 == 的否定，交换 true/false 跳转链
            std::mem::swap(&mut v1.t, &mut v1.f);
        }
        BinaryOperator::OpLt => code_comp(c, OpCode::Lt, v1, v2),
        BinaryOperator::OpLe => code_comp(c, OpCode::Le, v1, v2),
        BinaryOperator::OpGt => {
            // > 转换为 <
            code_comp(c, OpCode::Lt, v2, v1);
            *v1 = v2.clone();
        }
        BinaryOperator::OpGe => {
            // >= 转换为 <=
            code_comp(c, OpCode::Le, v2, v1);
            *v1 = v2.clone();
        }
        BinaryOperator::OpNop => {}
    }

    Ok(())
}

/// Compile closure expression (anonymous function) - 对齐body
fn compile_closure_expr(c: &mut Compiler, closure: &LuaClosureExpr) -> Result<ExpDesc, String> {
    // Create a child compiler for the nested function
    let parent_scope = c.scope_chain.clone();
    let vm_ptr = c.vm_ptr;
    let line_index = c.line_index;
    let current_line = c.last_line;

    let mut child_compiler = Compiler::new_with_parent(
        parent_scope,
        vm_ptr,
        line_index,
        current_line,
        Some(c as *mut Compiler),
    );

    // Compile function body
    compile_function_body(&mut child_compiler, closure)?;

    // Store the child chunk
    c.child_chunks.push(child_compiler.chunk);
    let proto_idx = c.child_chunks.len() - 1;

    // Generate CLOSURE instruction
    super::helpers::reserve_regs(c, 1);
    let reg = c.freereg - 1;
    let pc = super::helpers::code_abx(c, crate::lua_vm::OpCode::Closure, reg, proto_idx as u32);

    // Return expression descriptor for the closure
    let mut v = ExpDesc::new_void();
    v.kind = expdesc::ExpKind::VReloc;
    v.info = pc as u32;
    Ok(v)
}

/// Compile function body (parameters and block) - 对齐body
fn compile_function_body(child: &mut Compiler, closure: &LuaClosureExpr) -> Result<(), String> {
    // Enter function block
    enter_block(child, false)?;

    // Parse parameters
    if let Some(param_list) = closure.get_params_list() {
        let params = param_list.get_params();
        let mut param_count = 0;
        let mut has_vararg = false;

        for param in params {
            if param.is_dots() {
                has_vararg = true;
                break;
            } else if let Some(name_token) = param.get_name_token() {
                let name = name_token.get_name_text().to_string();
                new_localvar(child, name)?;
                param_count += 1;
            }
        }

        child.chunk.param_count = param_count;
        child.chunk.is_vararg = has_vararg;

        // Activate parameter variables
        adjustlocalvars(child, param_count);

        // Generate VARARGPREP if function is vararg
        if has_vararg {
            helpers::code_abc(
                child,
                OpCode::VarargPrep,
                param_count as u32,
                0,
                0,
            );
        }
    }

    // Compile function body
    if let Some(block) = closure.get_block() {
        compile_statlist(child, &block)?;
    }

    // Final return
    let first = helpers::nvarstack(child);
    helpers::ret(child, first, 0);

    // Leave function block
    leave_block(child)?;

    // Set max stack size
    if child.peak_freereg > child.chunk.max_stack_size as u32 {
        child.chunk.max_stack_size = child.peak_freereg as usize;
    }

    Ok(())
}
