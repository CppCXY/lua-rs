/// Load and Move instructions
///
/// These instructions handle loading constants and moving values between registers.
use crate::LuaValue;
use crate::lua_vm::{LuaCallFrame, LuaVM};
use crate::{get_a, get_ax, get_b, get_bx, get_sbx};

/// VARARGPREP A
/// Prepare stack for vararg function
/// A is the number of fixed parameters
///
/// This instruction moves vararg arguments to a safe location after max_stack_size,
/// so they won't be overwritten by local variable operations.
#[inline(always)]
pub fn exec_varargprep(
    vm: &mut LuaVM,
    instr: u32,
    frame_ptr: *mut LuaCallFrame,
    base_ptr: &mut usize,
) {
    let a = get_a!(instr); // number of fixed params

    let frame = unsafe { &*frame_ptr };
    let frame_base = frame.base_ptr as usize;
    let top = frame.top as usize;

    // OPTIMIZED: Use cached max_stack from frame instead of object_pool lookup
    let max_stack_size = frame.max_stack as usize;

    // Arguments were placed starting at frame_base by CALL instruction
    // Fixed parameters are at frame_base + 0 to frame_base + a - 1
    // Extra arguments (varargs) are at frame_base + a to frame_base + top - 1
    // We need to move the varargs to frame_base + max_stack_size to protect them
    // from being overwritten by local variable operations

    if top > a {
        let vararg_count = top - a;
        let vararg_dest = frame_base + max_stack_size;

        // Ensure we have enough space for the varargs
        let required_size = vararg_dest + vararg_count;
        vm.ensure_stack_capacity(required_size);

        // Move varargs from frame_base + a to frame_base + max_stack_size
        // OPTIMIZED: Use ptr::copy_nonoverlapping when safe, otherwise copy in reverse
        unsafe {
            let reg_ptr = vm.register_stack.as_mut_ptr();
            let src = frame_base + a;

            if vararg_dest >= src + vararg_count {
                // No overlap - use fast copy
                std::ptr::copy_nonoverlapping(
                    reg_ptr.add(src),
                    reg_ptr.add(vararg_dest),
                    vararg_count,
                );
            } else {
                // Overlapping - copy in reverse
                for i in (0..vararg_count).rev() {
                    *reg_ptr.add(vararg_dest + i) = *reg_ptr.add(src + i);
                }
            }
        }

        // Set vararg info in frame
        unsafe { &mut *frame_ptr }.set_vararg(vararg_dest, vararg_count);
    } else {
        // No varargs passed
        unsafe { &mut *frame_ptr }.set_vararg(frame_base + max_stack_size, 0);
    }

    // Initialize local variables (registers from a to max_stack_size) with nil
    // OPTIMIZED: Use bulk fill
    let nil_start = frame_base + a;
    let nil_end = (frame_base + max_stack_size).min(vm.register_stack.len());
    if nil_start < nil_end {
        let nil_val = LuaValue::nil();
        unsafe {
            let reg_ptr = vm.register_stack.as_mut_ptr();
            for i in nil_start..nil_end {
                *reg_ptr.add(i) = nil_val;
            }
        }
    }

    // updatebase - frame operations may change base_ptr
    unsafe {
        *base_ptr = (*frame_ptr).base_ptr as usize;
    }
}

/// LOADNIL A B
/// R[A], R[A+1], ..., R[A+B] := nil
#[inline(always)]
#[allow(dead_code)]
pub fn exec_loadnil(vm: &mut LuaVM, instr: u32, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);

    unsafe {
        let nil_val = LuaValue::nil();
        let reg_ptr = vm.register_stack.as_mut_ptr().add(base_ptr);
        for i in 0..=b {
            *reg_ptr.add(a + i) = nil_val;
        }
    }
}

/// LOADFALSE A
/// R[A] := false
#[inline(always)]
#[allow(dead_code)]
pub fn exec_loadfalse(vm: &mut LuaVM, instr: u32, base_ptr: usize) {
    let a = get_a!(instr);

    unsafe {
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::boolean(false);
    }
}

/// LFALSESKIP A
/// R[A] := false; pc++
#[inline(always)]
#[allow(dead_code)]
pub fn exec_lfalseskip(vm: &mut LuaVM, instr: u32, pc: &mut usize, base_ptr: usize) {
    let a = get_a!(instr);

    unsafe {
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::boolean(false);
        // Skip next instruction
        *pc += 1;
    }
}

/// LOADTRUE A
/// R[A] := true
#[inline(always)]
#[allow(dead_code)]
pub fn exec_loadtrue(vm: &mut LuaVM, instr: u32, base_ptr: usize) {
    let a = get_a!(instr);

    unsafe {
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::boolean(true);
    }
}

/// LOADI A sBx
/// R[A] := sBx (signed integer)
#[inline(always)]
#[allow(dead_code)]
#[allow(dead_code)]
pub fn exec_loadi(vm: &mut LuaVM, instr: u32, base_ptr: usize) {
    let a = get_a!(instr);
    let sbx = get_sbx!(instr);

    unsafe {
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::integer(sbx as i64);
    }
}

/// LOADF A sBx
/// R[A] := (lua_Number)sBx
#[inline(always)]
#[allow(dead_code)]
pub fn exec_loadf(vm: &mut LuaVM, instr: u32, base_ptr: usize) {
    let a = get_a!(instr);
    let sbx = get_sbx!(instr);

    unsafe {
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = LuaValue::number(sbx as f64);
    }
}

/// LOADK A Bx
/// R[A] := K[Bx]
/// OPTIMIZED: Uses cached constants_ptr for direct constant access
#[inline(always)]
#[allow(dead_code)]
pub fn exec_loadk(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, base_ptr: usize) {
    let a = get_a!(instr);
    let bx = get_bx!(instr);

    unsafe {
        // FAST PATH: Direct constant access via cached pointer
        let constant = *(*frame_ptr).constants_ptr.add(bx);
        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = constant;
    }
}

/// LOADKX A
/// R[A] := K[extra arg]
#[inline(always)]
#[allow(dead_code)]
pub fn exec_loadkx(vm: &mut LuaVM, instr: u32, frame_ptr: *mut LuaCallFrame, base_ptr: usize) {
    let a = get_a!(instr);

    unsafe {
        let pc_val = (*frame_ptr).pc as usize;
        (*frame_ptr).pc = (pc_val + 1) as u32; // Skip the extra arg instruction
        let func_id = (*frame_ptr).get_function_id();

        if let Some(fid) = func_id {
            if let Some(func_ref) = vm.object_pool.get_function(fid) {
                if let Some(&extra_instr) = func_ref.chunk.code.get(pc_val) {
                    let bx = get_ax!(extra_instr);
                    if let Some(&constant) = func_ref.chunk.constants.get(bx) {
                        *vm.register_stack.as_mut_ptr().add(base_ptr + a) = constant;
                    }
                }
            }
        }
    }
}

/// MOVE A B
/// R[A] := R[B]
#[inline(always)]
#[allow(dead_code)]
pub fn exec_move(vm: &mut LuaVM, instr: u32, base_ptr: usize) {
    let a = get_a!(instr);
    let b = get_b!(instr);

    unsafe {
        let reg_ptr = vm.register_stack.as_mut_ptr().add(base_ptr);
        *reg_ptr.add(a) = *reg_ptr.add(b);
    }
}
