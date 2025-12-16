
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
            // 匿名函数不是方法
            compile_closure_expr(c, closure_expr, false)
        }
        LuaExpr::CallExpr(call_expr) => {
            // Function call expression (对齐funcargs)
            compile_function_call(c, call_expr)
        }
        LuaExpr::TableExpr(table_expr) => {
            // Table constructor expression (对齐constructor)
            compile_table_constructor(c, table_expr)
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

    // 对齐官方fieldsel处理：先exp2anyregup确保t在寄存器或upvalue中
    // 但关键是：如果t已经是VIndexStr等延迟状态，exp2anyregup会通过discharge_vars生成指令
    // 然后indexed会基于这个新的VReloc/VNonReloc再次设置为VIndexStr
    super::exp2reg::exp2anyregup(c, &mut t);

    // Get the index/key
    if let Some(index_token) = index_expr.get_index_token() {
        // 注意：冒号语法在这里不报错，因为可能是在函数调用context中
        // 会在compile_function_call中特殊处理
        
        if index_token.is_dot() || index_token.is_colon() {
            // Dot/Colon notation: t.field 或 t:method
            // 注意：冒号在这里和点号处理相同，实际的SELF指令会在函数调用时生成
            if let Some(key) = index_expr.get_index_key() {
                let key_name = match key {
                    LuaIndexKey::Name(name_token) => name_token.get_name_text().to_string(),
                    _ => return Err("Dot/Colon notation requires name key".to_string()),
                };

                // Create string constant for field name (对齐luac，使用VKStr)
                let k_idx = helpers::string_k(c, key_name);
                let mut k = ExpDesc::new_kstr(k_idx);

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

    // 特殊处理移位和减法优化（对齐官方lcode.c luaK_posfix中的OPR_SUB/OPR_SHR/OPR_SHL case）
    
    // 1. SUB优化：x - n => ADDI x, -n + MMBINI x, n, TM_SUB（如果n fit sC）
    // 对齐lcode.c:1743: if (finishbinexpneg(fs, e1, e2, OP_ADDI, line, TM_SUB))
    if op == OpCode::Sub {
        if let ExpKind::VKInt = e2.kind {
            let val = e2.ival;
            if val >= -127 && val <= 128 && (-val) >= -127 && (-val) <= 128 {
                // 对齐finishbinexpval: exp2anyreg → 生成指令 → freeexps → 修改kind
                let o1 = super::exp2reg::exp2anyreg(c, e1);
                let imm = ((-val + 127) & 0xFF) as u32;
                let pc = helpers::code_abc(c, OpCode::AddI, 0, o1, imm);
                // 关键：先free，再改kind（此时e1还是VNonReloc，可以被free）
                super::exp2reg::free_exp(c, e1);
                super::exp2reg::free_exp(c, e2);
                e1.info = pc as u32;
                e1.kind = ExpKind::VReloc;
                // MMBINI: 第二个参数是原始值val（不是负值）
                let imm_mm = ((val + 128) & 0xFF) as u32;
                helpers::code_abc(c, OpCode::MmBinI, o1, imm_mm, 7); // TM_SUB=7
                return;
            }
        }
    }

    // 2. SHR优化：x >> n => SHRI x, n + MMBINI x, n, TM_SHR（如果n fit sC）
    // 对齐lcode.c:1760-1764
    if op == OpCode::Shr {
        if let ExpKind::VKInt = e2.kind {
            let val = e2.ival;
            if val >= -128 && val <= 127 {
                // 对齐codebini → finishbinexpval: exp2anyreg → 生成指令 → freeexps → 修改kind
                let o1 = super::exp2reg::exp2anyreg(c, e1);
                let imm = ((val + 127) & 0xFF) as u32;
                let pc = helpers::code_abc(c, OpCode::ShrI, 0, o1, imm);
                // 关键：先free，再改kind
                super::exp2reg::free_exp(c, e1);
                super::exp2reg::free_exp(c, e2);
                e1.info = pc as u32;
                e1.kind = ExpKind::VReloc;
                // MMBINI
                let imm_mm = ((val + 128) & 0xFF) as u32;
                helpers::code_abc(c, OpCode::MmBinI, o1, imm_mm, 17); // TM_SHR=17
                return;
            }
        }
    }

    // 3. SHL优化（对齐官方lcode.c:1746-1758）
    if op == OpCode::Shl {
        // 特殊情况1：I << x 使用SHLI（立即数在前）
        // 对齐lcode.c:1747-1750
        if e1.kind == ExpKind::VKInt {
            let val = e1.ival;
            if val >= -128 && val <= 127 {
                // swap e1 和 e2，对齐官方swapexps
                std::mem::swap(e1, e2);
                // 对齐codebini → finishbinexpval: exp2anyreg → 生成指令 → freeexps → 修改kind
                let o1 = super::exp2reg::exp2anyreg(c, e1);
                let imm = ((val + 127) & 0xFF) as u32;
                let pc = helpers::code_abc(c, OpCode::ShlI, 0, o1, imm);
                // 关键：先free，再改kind（此时e1是VNonReloc，可以被释放）
                super::exp2reg::free_exp(c, e1);
                super::exp2reg::free_exp(c, e2);
                e1.info = pc as u32;
                e1.kind = ExpKind::VReloc;
                // MMBINI: 参数是原始值val，flip=1
                let imm_mm = ((val + 128) & 0xFF) as u32;
                helpers::code_abck(c, OpCode::MmBinI, o1, imm_mm, 16, true); // TM_SHL=16
                return;
            }
        }
        // 特殊情况2：x << n => SHRI x, -n（如果n fit sC）
        // 对齐lcode.c:1751-1753
        if let ExpKind::VKInt = e2.kind {
            let val = e2.ival;
            if val >= -127 && val <= 128 && (-val) >= -128 && (-val) <= 127 {
                // 对齐finishbinexpneg: exp2anyreg → 生成指令 → freeexps → 修改kind
                let o1 = super::exp2reg::exp2anyreg(c, e1);
                let imm = ((-val + 127) & 0xFF) as u32;
                let pc = helpers::code_abc(c, OpCode::ShrI, 0, o1, imm);
                // 关键：先free，再改kind
                super::exp2reg::free_exp(c, e1);
                super::exp2reg::free_exp(c, e2);
                e1.info = pc as u32;
                e1.kind = ExpKind::VReloc;
                // MMBINI: 参数是原始值val，元方法是TM_SHL
                let imm_mm = ((val + 128) & 0xFF) as u32;
                helpers::code_abc(c, OpCode::MmBinI, o1, imm_mm, 16); // TM_SHL=16
                return;
            }
        }
    }

    // 标准路径：左操作数总是要放到寄存器
    let o1 = super::exp2reg::exp2anyreg(c, e1);
    
    // 检查是否可以使用K后缀指令（对齐Lua 5.4 codebinarith）
    let (final_op, o2, use_k) = if can_use_k_variant(op) {
        // 检查右操作数是否为常量
        if let Some(k) = try_get_k_value(c, e2) {
            // 使用K后缀指令，o2是常量索引
            (get_k_variant(op), k, true)
        } else {
            // 右操作数不是常量，使用普通指令
            let o2 = super::exp2reg::exp2anyreg(c, e2);
            (op, o2, false)
        }
    } else {
        // 不支持K变体，右操作数必须在寄存器
        let o2 = super::exp2reg::exp2anyreg(c, e2);
        (op, o2, false)
    };

    // 释放表达式
    if o1 > o2 {
        super::exp2reg::free_exp(c, e1);
        super::exp2reg::free_exp(c, e2);
    } else {
        super::exp2reg::free_exp(c, e2);
        super::exp2reg::free_exp(c, e1);
    }

    // 生成指令
    e1.info = helpers::code_abck(c, final_op, 0, o1, o2, use_k) as u32;
    e1.kind = ExpKind::VReloc;
    
    // 生成元方法标记指令（对齐 Lua 5.4 codeMMBin）
    if use_k {
        code_mmbink(c, final_op, o1, o2);
    } else {
        code_mmbin(c, final_op, o1, o2);
    }
}

/// 检查操作是否支持K后缀变体
fn can_use_k_variant(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div | 
        OpCode::IDiv | OpCode::Mod | OpCode::Pow |
        OpCode::BAnd | OpCode::BOr | OpCode::BXor |
        OpCode::Shl | OpCode::Shr
    )
}

/// 获取K后缀操作码
fn get_k_variant(op: OpCode) -> OpCode {
    match op {
        OpCode::Add => OpCode::AddK,
        OpCode::Sub => OpCode::SubK,
        OpCode::Mul => OpCode::MulK,
        OpCode::Div => OpCode::DivK,
        OpCode::IDiv => OpCode::IDivK,
        OpCode::Mod => OpCode::ModK,
        OpCode::Pow => OpCode::PowK,
        OpCode::BAnd => OpCode::BAndK,
        OpCode::BOr => OpCode::BOrK,
        OpCode::BXor => OpCode::BXorK,
        OpCode::Shl => OpCode::ShlI,  // 注意：移位用整数立即数
        OpCode::Shr => OpCode::ShrI,
        _ => op,
    }
}

/// 尝试获取表达式的常量值索引
/// 对齐官方codearith中的tonumeral检查：只有数值常量才能作为K操作数
fn try_get_k_value(c: &mut Compiler, e: &mut ExpDesc) -> Option<u32> {
    match e.kind {
        // 注意：只有VKInt和VKFlt可以作为算术/位运算的K操作数
        // 字符串常量(VK/VKStr)不能直接用于K变体指令
        // 参考lcode.c:1505: if (tonumeral(e2, NULL) && luaK_exp2K(fs, e2))
        ExpKind::VKInt => {
            // 整数常量，添加到常量表
            Some(super::helpers::int_k(c, e.ival))
        }
        ExpKind::VKFlt => {
            // 浮点常量，添加到常量表
            Some(super::helpers::number_k(c, e.nval))
        }
        _ => None,
    }
}

/// 生成元方法二元操作标记（对齐 luaK_codeMMBin in lcode.c）
fn code_mmbin(c: &mut Compiler, op: OpCode, o1: u32, o2: u32) {
    let mm = get_mm_index(op);
    if mm > 0 {
        use super::helpers::code_abc;
        code_abc(c, OpCode::MmBin, o1, o2, mm);
    }
}

/// 生成带常量的元方法二元操作标记（对齐 luaK_codeMMBinK）
fn code_mmbink(c: &mut Compiler, op: OpCode, o1: u32, k: u32) {
    let mm = get_mm_index(op);
    if mm > 0 {
        use super::helpers::code_abc;
        code_abc(c, OpCode::MmBinK, o1, k, mm);
    }
}

/// 获取元方法索引
fn get_mm_index(op: OpCode) -> u32 {
    // 将OpCode映射到元方法ID（参考Lua 5.4 ltm.h中的TM enum）
    // ltm.h定义：TM_INDEX(0), TM_NEWINDEX(1), TM_GC(2), TM_MODE(3), TM_LEN(4), TM_EQ(5),
    // TM_ADD(6), TM_SUB(7), TM_MUL(8), TM_MOD(9), TM_POW(10), TM_DIV(11), TM_IDIV(12),
    // TM_BAND(13), TM_BOR(14), TM_BXOR(15), TM_SHL(16), TM_SHR(17), ...
    match op {
        OpCode::Add | OpCode::AddK => 6,      // TM_ADD
        OpCode::Sub | OpCode::SubK => 7,      // TM_SUB
        OpCode::Mul | OpCode::MulK => 8,      // TM_MUL
        OpCode::Mod | OpCode::ModK => 9,      // TM_MOD
        OpCode::Pow | OpCode::PowK => 10,     // TM_POW
        OpCode::Div | OpCode::DivK => 11,     // TM_DIV
        OpCode::IDiv | OpCode::IDivK => 12,    // TM_IDIV
        OpCode::BAnd | OpCode::BAndK => 13,    // TM_BAND
        OpCode::BOr | OpCode::BOrK => 14,     // TM_BOR
        OpCode::BXor | OpCode::BXorK => 15,    // TM_BXOR
        OpCode::Shl | OpCode::ShlI => 16,     // TM_SHL
        OpCode::Shr | OpCode::ShrI => 17,     // TM_SHR
        _ => 0,                               // 其他操作不需要元方法
    }
}

/// 生成比较指令（对齐 codecomp）
/// inv参数表示是否反转条件（用于~=）
fn code_comp(c: &mut Compiler, op: OpCode, e1: &mut ExpDesc, e2: &mut ExpDesc, inv: bool) {
    use super::helpers;

    // 左操作数总是在寄存器
    let o1 = super::exp2reg::exp2anyreg(c, e1);
    
    // 检查是否可以使用EQI或EQK优化（对齐Lua 5.4 lcode.c:1369-1386 codeeq函数）
    // 只有EQ操作支持EQI/EQK指令
    if op == OpCode::Eq {
        // 首先检查是否可以用EQI（立即数形式）
        // 对齐官方lcode.c:1377: if (isSCnumber(e2, &im, &isfloat))
        // isSCnumber检查是否是可以fit到sC字段的整数（-128到127）
        if let ExpKind::VKInt = e2.kind {
            let val = e2.ival;
            // fitsC: (l_castS2U(i) + OFFSET_sC <= cast_uint(MAXARG_C))
            // OFFSET_sC = 128, MAXARG_C = 255, 所以范围是-128到127
            if val >= -128 && val <= 127 {
                // 使用EQI指令：EQI A sB k，比较R[A]和立即数sB
                super::exp2reg::free_exp(c, e1);
                // int2sC(i) = (i) + OFFSET_sC = val + 128
                let imm = ((val + 128) & 0xFF) as u32;
                // 生成EQI指令，k位对齐官方：condjump(fs, op, r1, r2, isfloat, (opr == OPR_EQ))
                // 即：对于==，k=1（!inv）；对于~=，k=0（inv）
                helpers::code_abck(c, OpCode::EqI, o1, imm, 0, !inv);
                let jmp = helpers::jump(c);
                e1.info = jmp as u32;
                e1.kind = ExpKind::VJmp;
                return;
            }
        }
        
        // 然后尝试将右操作数转换为常量（EQK）
        // 对齐官方lcode.c:1381: else if (exp2RK(fs, e2))
        if super::exp2reg::exp2k(c, e2) {
            // 使用EQK指令：EQK A B k，比较R[A]和K[B]
            super::exp2reg::free_exp(c, e1);
            let k_idx = e2.info;
            // 生成EQK指令，k位对齐官方：对于==，k=1；对于~=，k=0
            helpers::code_abck(c, OpCode::EqK, o1, k_idx, 0, !inv);
            let jmp = helpers::jump(c);
            e1.info = jmp as u32;
            e1.kind = ExpKind::VJmp;
            return;
        }
    }
    
    // 标准路径：两个操作数都在寄存器
    let o2 = super::exp2reg::exp2anyreg(c, e2);

    // 对齐官方freeexps：按从大到小顺序释放寄存器（先释放高寄存器，再释放低寄存器）
    // 这样确保freereg正确回退
    if o1 > o2 {
        super::exp2reg::free_exp(c, e1);
        super::exp2reg::free_exp(c, e2);
    } else {
        super::exp2reg::free_exp(c, e2);
        super::exp2reg::free_exp(c, e1);
    }

    // 生成比较指令（结果是跳转）
    // 对齐官方lcode.c:1608: e1->u.info = condjump(fs, op, r1, r2, isfloat, (opr == OPR_EQ));
    // 对于EQ: k=1(相等时跳转), inv=false表示==, inv=true表示~=
    // 对于其他比较(LT/LE): k=1(条件为真时跳转)
    let k = if op == OpCode::Eq { !inv } else { true };
    e1.info = helpers::cond_jump(c, op, o1, o2, k) as u32;
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
            // 官方实现：不discharge，直接连接跳转列表
            helpers::concat(c, &mut v2.f, v1.f);
            *v1 = v2.clone();
        }
        BinaryOperator::OpOr => {
            // or: v1 or v2
            debug_assert!(v1.f == helpers::NO_JUMP); // 左操作数为 false 时继续
            // 官方实现：不discharge，直接连接跳转列表
            helpers::concat(c, &mut v2.t, v1.t);
            *v1 = v2.clone();
        }
        BinaryOperator::OpConcat => {
            // 字符串连接: v1 .. v2
            // 关键：检查 v1（左操作数）是否是 CONCAT，而不是 v2
            // 因为 AST 遍历时，对于 (a .. b) .. c，先处理左边生成 CONCAT，再处理右边
            super::exp2reg::exp2val(c, v2);
            if v1.kind == ExpKind::VReloc && helpers::get_op(c, v1.info) == OpCode::Concat {
                // 合并优化：左边是 CONCAT，增加 B 字段的值数量
                // v1 是 CONCAT A B，现在要加上 v2，所以 B += 1
                super::exp2reg::exp2nextreg(c, v2);
                let concat_pc = v1.info;
                let old_b = helpers::getarg_b(c, concat_pc);
                helpers::setarg_b(c, concat_pc, old_b + 1);
                // v1 保持不变（仍然指向同一条 CONCAT 指令）
            } else {
                // 生成新的 CONCAT：A=v1寄存器, B=2（连接2个值）
                super::exp2reg::exp2nextreg(c, v2);
                let reg1 = v1.info;
                let reg2 = v2.info;
                // 确保寄存器连续（infix 阶段已经 exp2nextreg）
                debug_assert!(reg2 == reg1 + 1, "CONCAT registers not consecutive: {} and {}", reg1, reg2);
                // 释放寄存器
                super::exp2reg::free_exp(c, v2);
                super::exp2reg::free_exp(c, v1);
                // 生成 CONCAT A 2
                v1.info = helpers::code_abc(c, OpCode::Concat, reg1, 2, 0) as u32;
                v1.kind = ExpKind::VReloc;
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
        BinaryOperator::OpEq => code_comp(c, OpCode::Eq, v1, v2, false),
        BinaryOperator::OpNe => {
            code_comp(c, OpCode::Eq, v1, v2, true);
            // ~= 是 == 的否定，对于EQK k位已经设置，对于EQ需要交换跳转链
            std::mem::swap(&mut v1.t, &mut v1.f);
        }
        BinaryOperator::OpLt => code_comp(c, OpCode::Lt, v1, v2, false),
        BinaryOperator::OpLe => code_comp(c, OpCode::Le, v1, v2, false),
        BinaryOperator::OpGt => {
            // > 转换为 <
            code_comp(c, OpCode::Lt, v2, v1, false);
            *v1 = v2.clone();
        }
        BinaryOperator::OpGe => {
            // >= 转换为 <=
            code_comp(c, OpCode::Le, v2, v1, false);
            *v1 = v2.clone();
        }
        BinaryOperator::OpNop => {}
    }

    Ok(())
}

/// Compile closure expression (anonymous function) - 对齐body
pub(crate) fn compile_closure_expr(c: &mut Compiler, closure: &LuaClosureExpr, ismethod: bool) -> Result<ExpDesc, String> {
    // Create a child compiler for the nested function
    let parent_scope = c.scope_chain.clone();
    let vm_ptr = c.vm_ptr;
    let line_index = c.line_index;
    let source = c.source;
    let chunk_name = c.chunk_name.clone();
    let current_line = c.last_line;

    let mut child_compiler = Compiler::new_with_parent(
        parent_scope,
        vm_ptr,
        line_index,
        source,
        &chunk_name,
        current_line,
        Some(c as *mut Compiler),
    );

    // Compile function body with ismethod flag
    compile_function_body(&mut child_compiler, closure, ismethod)?;

    // Get upvalue information from child before moving chunk
    let upvalue_descs = {
        let scope = child_compiler.scope_chain.borrow();
        scope.upvalues.clone()
    };
    let num_upvalues = upvalue_descs.len();
    
    // Store upvalue descriptors in child chunk (对齐luac的Proto.upvalues)
    child_compiler.chunk.upvalue_count = num_upvalues;
    child_compiler.chunk.upvalue_descs = upvalue_descs.iter().map(|uv| {
        crate::lua_value::UpvalueDesc {
            is_local: uv.is_local,
            index: uv.index,
        }
    }).collect();
    
    // Store the child chunk
    c.child_chunks.push(child_compiler.chunk);
    let proto_idx = c.child_chunks.len() - 1;

    // Generate CLOSURE instruction (对齐 luaK_codeclosure)
    super::helpers::reserve_regs(c, 1);
    let reg = c.freereg - 1;
    super::helpers::code_abx(c, crate::lua_vm::OpCode::Closure, reg, proto_idx as u32);

    // Generate upvalue initialization instructions (对齐luac的codeclosure)
    // After CLOSURE, we need to emit instructions to describe how to capture each upvalue
    // 在luac 5.4中，这些信息已经在upvalue_descs中，VM会根据它来捕获upvalues
    // 但我们仍然需要确保upvalue_descs已正确设置
    
    // Return expression descriptor (already in register after reserve_regs)
    let mut v = ExpDesc::new_void();
    v.kind = expdesc::ExpKind::VNonReloc;
    v.info = reg;
    Ok(v)
}

/// Compile function body (parameters and block) - 对齐body
fn compile_function_body(child: &mut Compiler, closure: &LuaClosureExpr, ismethod: bool) -> Result<(), String> {
    // Enter function block
    enter_block(child, false)?;

    // If method, create 'self' parameter first (对齐 lparser.c body函数)
    if ismethod {
        new_localvar(child, "self".to_string())?;
        adjustlocalvars(child, 1);
    }

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

        // 如果是方法，param_count需要加1（包含self）
        if ismethod {
            param_count += 1;
        }

        child.chunk.param_count = param_count;
        child.chunk.is_vararg = has_vararg;

        // Activate parameter variables
        adjustlocalvars(child, param_count - if ismethod { 1 } else { 0 });
        
        // Reserve registers for parameters (对齐luaK_reserveregs)
        helpers::reserve_regs(child, child.nactvar as u32);

        // Generate VARARGPREP if function is vararg
        if has_vararg {
            helpers::code_abc(child, OpCode::VarargPrep, param_count as u32, 0, 0);
        }
    }

    // Compile function body
    if let Some(block) = closure.get_block() {
        compile_statlist(child, &block)?;
    }

    // Final return
    let first = helpers::nvarstack(child);
    helpers::ret(child, first, 0);

    // Store local variable names for debug info BEFORE leaving block
    {
        let scope = child.scope_chain.borrow();
        child.chunk.locals = scope.locals.iter().map(|l| l.name.clone()).collect();
    }

    // Leave function block
    leave_block(child)?;

    // Set max stack size
    if child.peak_freereg > child.chunk.max_stack_size as u32 {
        child.chunk.max_stack_size = child.peak_freereg as usize;
    }

    // 对齐luaK_finish: 最后调整RETURN/TAILCALL指令的k位和C字段
    helpers::finish(child);

    Ok(())
}

/// Convert expression to RK operand (对齐exp2RK)
/// Returns true if expression is K (constant), false if in register
fn exp2rk(c: &mut Compiler, e: &mut ExpDesc) -> bool {
    // Try to make it a constant
    if super::exp2reg::exp2k(c, e) {
        true
    } else {
        // Put in register
        super::exp2reg::exp2anyreg(c, e);
        false
    }
}

/// Code ABRK instruction format (对齐codeABRK)
/// 这是官方Lua中codeABRK的实现，用于生成SETTABUP/SETFIELD/SETI/SETTABLE等指令
/// 它会尝试将表达式转换为常量，如果失败则放到寄存器
pub(crate) fn code_abrk(c: &mut Compiler, op: OpCode, a: u32, b: u32, ec: &mut ExpDesc) {
    let k = exp2rk(c, ec);
    let c_val = ec.info;
    helpers::code_abck(c, op, a, b, c_val, k);
}

/// Emit SELF instruction (对齐luaK_self)
/// Converts expression 'e' into 'e:key(e,'
/// SELF A B C: R(A+1) := R(B); R(A) := R(B)[RK(C)]
fn code_self(c: &mut Compiler, e: &mut ExpDesc, key: &mut ExpDesc) -> u32 {
    // Ensure object is in a register
    super::exp2reg::exp2anyreg(c, e);
    let ereg = e.info; // register where object was placed
    
    // Free the object register since SELF will use new registers
    helpers::freeexp(c, e);
    
    // Allocate base register for SELF
    e.info = c.freereg;
    e.kind = expdesc::ExpKind::VNonReloc;
    
    // Reserve 2 registers: one for method, one for self parameter
    helpers::reserve_regs(c, 2);
    
    // Generate SELF instruction
    code_abrk(c, OpCode::Self_, e.info, ereg, key);
    
    // Free key expression
    helpers::freeexp(c, key);
    
    e.info
}

/// Compile function call expression - 对齐funcargs
fn compile_function_call(c: &mut Compiler, call_expr: &LuaCallExpr) -> Result<ExpDesc, String> {
    use super::exp2reg;

    // Get the prefix expression (function to call)
    let prefix = call_expr
        .get_prefix_expr()
        .ok_or("call expression missing prefix")?;

    // 检查是否是冒号方法调用 (obj:method())
    // 如果prefix是IndexExpr且使用冒号，需要生成SELF指令
    let is_method_call = if let LuaExpr::IndexExpr(index_expr) = &prefix {
        if let Some(index_token) = index_expr.get_index_token() {
            index_token.is_colon()
        } else {
            false
        }
    } else {
        false
    };

    let base = if is_method_call {
        // 方法调用：obj:method(args) 转换为 obj.method(obj, args)
        // 使用SELF指令：SELF A B C  =>  R(A+1):=R(B); R(A):=R(B)[RK(C)]
        if let LuaExpr::IndexExpr(index_expr) = &prefix {
            // 编译对象表达式
            let obj_prefix = index_expr.get_prefix_expr()
                .ok_or("method call missing object")?;
            let mut obj = expr(c, &obj_prefix)?;
            
            // 获取方法名
            let method_name = if let Some(key) = index_expr.get_index_key() {
                match key {
                    LuaIndexKey::Name(name_token) => name_token.get_name_text().to_string(),
                    _ => return Err("Method call requires name key".to_string()),
                }
            } else {
                return Err("Method call missing method name".to_string());
            };
            
            // 创建方法名的字符串常量
            let k_idx = super::helpers::string_k(c, method_name);
            let mut key = ExpDesc::new_kstr(k_idx);
            
            // 生成SELF指令 (对齐luaK_self)
            code_self(c, &mut obj, &mut key)
        } else {
            unreachable!("Checked is_method_call but not IndexExpr");
        }
    } else {
        // 普通函数调用
        // 参考lparser.c:1040 funcargs中的prefixexp处理
        let mut func = expr(c, &prefix)?;
        // 必须调用exp2nextreg确保函数在freereg位置
        // 这样后续参数才能正确放在func+1, func+2...
        exp2reg::exp2nextreg(c, &mut func);
        func.info as u32
    };

    // Get argument list
    let args = call_expr
        .get_args_list()
        .ok_or("call expression missing arguments")?
        .get_args()
        .collect::<Vec<_>>();
    // 方法调用时，self参数已经由SELF指令放入R(A+1)，所以参数从1开始
    let mut nargs = if is_method_call { 1i32 } else { 0i32 };

    // Compile each argument
    for (i, arg) in args.iter().enumerate() {
        let mut e = expr(c, &arg)?;

        // Last argument might be multi-return (call or vararg)
        if i == args.len() - 1
            && matches!(e.kind, expdesc::ExpKind::VCall | expdesc::ExpKind::VVararg)
        {
            // Set to return all values
            exp2reg::set_returns(c, &mut e, -1);
            nargs = -1; // Indicate variable number of args
        } else {
            exp2reg::exp2nextreg(c, &mut e);
            nargs += 1;
        }
    }

    // Generate CALL instruction
    let line = c.last_line;
    c.chunk.line_info.push(line);

    let b = if nargs == -1 { 0 } else { (nargs + 1) as u32 };
    let pc = super::helpers::code_abc(c, crate::lua_vm::OpCode::Call, base, b, 2); // C=2: want 1 result

    // Free registers after the call
    c.freereg = base + 1;

    // Return call expression descriptor
    let mut v = ExpDesc::new_void();
    v.kind = expdesc::ExpKind::VCall;
    v.info = pc as u32;
    Ok(v)
}

/// Compile table constructor - 对齐constructor
fn compile_table_constructor(
    c: &mut Compiler,
    table_expr: &LuaTableExpr,
) -> Result<ExpDesc, String> {
    use super::exp2reg;
    use super::helpers;

    // Allocate register for the table
    let reg = c.freereg;
    helpers::reserve_regs(c, 1);

    // Generate NEWTABLE instruction (对齐官方实现：立即生成 EXTRAARG 占位)
    let pc = helpers::code_abc(c, crate::lua_vm::OpCode::NewTable, reg, 0, 0);
    // 立即生成 EXTRAARG 占位（官方总是生成 NEWTABLE + EXTRAARG 对）
    helpers::code_ax(c, crate::lua_vm::OpCode::ExtraArg, 0);

    // Get table fields
    let fields = table_expr.get_fields();

    let mut narr = 0; // Array elements count
    let mut nhash = 0; // Hash elements count
    let mut tostore = 0; // Pending array elements to store

    for field in fields {
        if field.is_value_field() {
            if let Some(value_expr) = field.get_value_expr() {
                let mut v = expr(c, &value_expr)?;

                // Check if last field and is multi-return
                if matches!(v.kind, expdesc::ExpKind::VCall | expdesc::ExpKind::VVararg) {
                    // Last field with multi-return - set all returns
                    exp2reg::set_returns(c, &mut v, -1);

                    // Generate SETLIST for pending elements
                    if tostore > 0 {
                        helpers::code_abc(
                            c,
                            crate::lua_vm::OpCode::SetList,
                            reg,
                            tostore,
                            narr / 50 + 1,
                        );
                        tostore = 0;
                    }

                    // SETLIST with C=0 to store all remaining values
                    helpers::code_abc(c, crate::lua_vm::OpCode::SetList, reg, 0, narr / 50 + 1);
                    break;
                } else {
                    exp2reg::exp2nextreg(c, &mut v);
                    narr += 1;
                    tostore += 1;

                    // Flush if we have 50 elements (LFIELDS_PER_FLUSH)
                    if tostore >= 50 {
                        helpers::code_abc(
                            c,
                            crate::lua_vm::OpCode::SetList,
                            reg,
                            tostore,
                            narr / 50,
                        );
                        tostore = 0;
                        c.freereg = reg + 1;
                    }
                }
            } else {
                if let Some(index_key) = field.get_field_key() {
                    match index_key {
                        LuaIndexKey::Expr(key_expr) => {
                            let mut k = expr(c, &key_expr)?;
                            exp2reg::exp2val(c, &mut k);

                            if let Some(value_expr) = field.get_value_expr() {
                                let mut v = expr(c, &value_expr)?;
                                exp2reg::exp2val(c, &mut v);

                                // Generate SETTABLE instruction
                                super::exp2reg::discharge_2any_reg(c, &mut k);
                                super::exp2reg::discharge_2any_reg(c, &mut v);
                                helpers::code_abc(
                                    c,
                                    crate::lua_vm::OpCode::SetTable,
                                    reg,
                                    k.info,
                                    v.info,
                                );
                                nhash += 1;
                            }
                        }
                        LuaIndexKey::Name(name_token) => {
                            let key_name = name_token.get_name_text().to_string();
                            let k_idx = helpers::string_k(c, key_name);
                            let mut k = ExpDesc::new_k(k_idx);

                            if let Some(value_expr) = field.get_value_expr() {
                                let mut v = expr(c, &value_expr)?;
                                exp2reg::exp2val(c, &mut v);

                                // Generate SETTABLE instruction
                                super::exp2reg::discharge_2any_reg(c, &mut k);
                                super::exp2reg::discharge_2any_reg(c, &mut v);
                                helpers::code_abc(
                                    c,
                                    crate::lua_vm::OpCode::SetTable,
                                    reg,
                                    k.info,
                                    v.info,
                                );
                                nhash += 1;
                            }
                        }
                        LuaIndexKey::Integer(i) => {
                            let mut k = ExpDesc::new_int(i.get_int_value());

                            if let Some(value_expr) = field.get_value_expr() {
                                let mut v = expr(c, &value_expr)?;
                                exp2reg::exp2val(c, &mut v);

                                // Generate SETTABLE instruction
                                super::exp2reg::discharge_2any_reg(c, &mut k);
                                super::exp2reg::discharge_2any_reg(c, &mut v);
                                helpers::code_abc(
                                    c,
                                    crate::lua_vm::OpCode::SetTable,
                                    reg,
                                    k.info,
                                    v.info,
                                );
                                nhash += 1;
                            }
                        }
                        LuaIndexKey::String(string_token) => {
                            let str_val = string_token.get_value();
                            let k_idx = helpers::string_k(c, str_val.to_string());
                            let mut k = ExpDesc::new_k(k_idx);

                            if let Some(value_expr) = field.get_value_expr() {
                                let mut v = expr(c, &value_expr)?;
                                exp2reg::exp2val(c, &mut v);

                                // Generate SETTABLE instruction
                                super::exp2reg::discharge_2any_reg(c, &mut k);
                                super::exp2reg::discharge_2any_reg(c, &mut v);
                                helpers::code_abc(
                                    c,
                                    crate::lua_vm::OpCode::SetTable,
                                    reg,
                                    k.info,
                                    v.info,
                                );
                                nhash += 1;
                            }
                        }
                        _ => {
                            return Err("Invalid table field key".to_string());
                        }
                    }
                }
            }
        }
    }

    // Flush remaining array elements
    if tostore > 0 {
        helpers::code_abc(
            c,
            crate::lua_vm::OpCode::SetList,
            reg,
            tostore,
            narr / 50 + 1,
        );
    }

    // Update NEWTABLE instruction with size hints and EXTRAARG (对齐luaK_settablesize)
    // 官方实现：总是生成 NEWTABLE + EXTRAARG 两条指令
    helpers::set_table_size(c, pc, reg, narr, nhash);

    // Reset free register
    c.freereg = reg + 1;

    // Return table expression descriptor
    let mut v = ExpDesc::new_void();
    v.kind = expdesc::ExpKind::VNonReloc;
    v.info = reg;
    Ok(v)
}
