#[macro_export]
macro_rules! op_arithI {
    ($instr:expr, $lua_state:expr, $base:expr, $pc:expr, $iop:expr, $fop:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let sc = $instr.get_sc();

        unsafe {
            let sp = $lua_state.stack_mut().as_mut_ptr();
            let ra_ptr = sp.add($base + a);
            let v1_ptr = sp.add($base + b) as *const LuaValue;

            if pttisinteger(v1_ptr) {
                let iv1 = pivalue(v1_ptr);
                $pc += 1;
                psetivalue(ra_ptr, $iop(iv1, sc));
            } else if pttisfloat(v1_ptr) {
                let nb = pfltvalue(v1_ptr);
                let fimm = sc as f64;
                $pc += 1;
                psetfltvalue(ra_ptr, $fop(nb, fimm));
            }
        }
    }};
}

#[macro_export]
macro_rules! op_arithf_aux {
    ($ra_ptr:expr, $pc:expr, $v1_ptr:expr, $v2_ptr:expr, $fop:expr) => {{
        let mut n1 = 0.0;
        let mut n2 = 0.0;
        if ptonumberns($v1_ptr, &mut n1) && ptonumberns($v2_ptr, &mut n2) {
            $pc += 1;
            psetfltvalue($ra_ptr, $fop(n1, n2));
        }
    }};
}

#[macro_export]
macro_rules! op_arith {
    ($instr:expr, $lua_state:expr, $base:expr, $pc:expr, $iop:expr, $fop:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;

        unsafe {
            let sp = $lua_state.stack_mut().as_mut_ptr();
            let v1_ptr = sp.add($base + b) as *const LuaValue;
            let v2_ptr = sp.add($base + c) as *const LuaValue;
            let ra_ptr = sp.add($base + a);

            if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                let i1 = pivalue(v1_ptr);
                let i2 = pivalue(v2_ptr);
                $pc += 1;
                psetivalue(ra_ptr, $iop(i1, i2));
            } else {
                op_arithf_aux!(ra_ptr, $pc, v1_ptr, v2_ptr, $fop);
            }
        }
    }};
}

#[macro_export]
macro_rules! op_arithf {
    ($instr:expr, $lua_state:expr, $base:expr, $pc:expr, $fop:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;

        unsafe {
            let sp = $lua_state.stack_mut().as_mut_ptr();
            let v1_ptr = sp.add($base + b) as *const LuaValue;
            let v2_ptr = sp.add($base + c) as *const LuaValue;
            let ra_ptr = sp.add($base + a);
            op_arithf_aux!(ra_ptr, $pc, v1_ptr, v2_ptr, $fop);
        }
    }};
}

#[macro_export]
macro_rules! op_arithK {
    ($instr:expr, $lua_state:expr, $base:expr, $pc:expr, $constants:expr, $iop:expr, $fop:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;

        unsafe {
            let sp = $lua_state.stack_mut().as_mut_ptr();
            let v1_ptr = sp.add($base + b) as *const LuaValue;
            let v2_ptr = $constants.as_ptr().add(c);
            let ra_ptr = sp.add($base + a);

            if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                let i1 = pivalue(v1_ptr);
                let i2 = pivalue(v2_ptr);
                $pc += 1;
                psetivalue(ra_ptr, $iop(i1, i2));
            } else {
                op_arithf_aux!(ra_ptr, $pc, v1_ptr, v2_ptr, $fop);
            }
        }
    }};
}

#[macro_export]
macro_rules! op_arithfK {
    ($instr:expr, $lua_state:expr, $base:expr, $pc:expr, $constants:expr, $fop:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;

        unsafe {
            let sp = $lua_state.stack_mut().as_mut_ptr();
            let v1_ptr = sp.add($base + b) as *const LuaValue;
            let v2_ptr = $constants.as_ptr().add(c);
            let ra_ptr = sp.add($base + a);
            op_arithf_aux!(ra_ptr, $pc, v1_ptr, v2_ptr, $fop);
        }
    }};
}

#[macro_export]
macro_rules! op_arith_check_zero {
    ($instr:expr, $lua_state:expr, $ci:expr, $base:expr, $pc:expr, $iop:expr, $fop:expr, $err_fn:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;

        unsafe {
            let sp = $lua_state.stack_mut().as_mut_ptr();
            let v1_ptr = sp.add($base + b) as *const LuaValue;
            let v2_ptr = sp.add($base + c) as *const LuaValue;
            let ra_ptr = sp.add($base + a);

            if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                let i1 = pivalue(v1_ptr);
                let i2 = pivalue(v2_ptr);
                if i2 != 0 {
                    $pc += 1;
                    psetivalue(ra_ptr, $iop(i1, i2));
                } else {
                    $ci.save_pc($pc);
                    return Err($err_fn($lua_state));
                }
            } else {
                op_arithf_aux!(ra_ptr, $pc, v1_ptr, v2_ptr, $fop);
            }
        }
    }};
}

#[macro_export]
macro_rules! op_arithK_check_zero {
    ($instr:expr, $lua_state:expr, $ci:expr, $base:expr, $pc:expr, $constants:expr, $iop:expr, $fop:expr, $err_fn:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;

        unsafe {
            let sp = $lua_state.stack_mut().as_mut_ptr();
            let v1_ptr = sp.add($base + b) as *const LuaValue;
            let v2_ptr = $constants.as_ptr().add(c);
            let ra_ptr = sp.add($base + a);

            if pttisinteger(v1_ptr) && pttisinteger(v2_ptr) {
                let i1 = pivalue(v1_ptr);
                let i2 = pivalue(v2_ptr);
                if i2 != 0 {
                    $pc += 1;
                    psetivalue(ra_ptr, $iop(i1, i2));
                } else {
                    $ci.save_pc($pc);
                    return Err($err_fn($lua_state));
                }
            } else {
                op_arithf_aux!(ra_ptr, $pc, v1_ptr, v2_ptr, $fop);
            }
        }
    }};
}

#[macro_export]
macro_rules! op_bitwise {
    ($instr:expr, $lua_state:expr, $base:expr, $pc:expr, $op:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;

        unsafe {
            let sp = $lua_state.stack_mut().as_mut_ptr();
            let v1_ptr = sp.add($base + b) as *const LuaValue;
            let v2_ptr = sp.add($base + c) as *const LuaValue;

            let mut i1 = 0i64;
            let mut i2 = 0i64;
            if tointegerns(&*v1_ptr, &mut i1) && tointegerns(&*v2_ptr, &mut i2) {
                let ra_ptr = sp.add($base + a);
                $pc += 1;
                psetivalue(ra_ptr, $op(i1, i2));
            }
        }
    }};
}

#[macro_export]
macro_rules! op_bitwiseK {
    ($instr:expr, $lua_state:expr, $base:expr, $pc:expr, $constants:expr, $op:expr) => {{
        let a = $instr.get_a() as usize;
        let b = $instr.get_b() as usize;
        let c = $instr.get_c() as usize;

        unsafe {
            let sp = $lua_state.stack_mut().as_mut_ptr();
            let v1_ptr = sp.add($base + b) as *const LuaValue;
            let v2_ptr = $constants.as_ptr().add(c);

            let mut i1 = 0i64;
            let i2 = pivalue(v2_ptr);
            if tointegerns(&*v1_ptr, &mut i1) {
                let ra_ptr = sp.add($base + a);
                $pc += 1;
                psetivalue(ra_ptr, $op(i1, i2));
            }
        }
    }};
}
