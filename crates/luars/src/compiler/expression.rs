// Port of expdesc from lcode.h
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpKind {
    VVOID,  // when 'expdesc' describes the last expression of a list, this kind means an empty list
    VNIL,   // constant nil
    VTRUE,  // constant true
    VFALSE, // constant false
    VK,     // constant in 'k'; info = index of constant in 'k'
    VKFLT,  // floating constant; nval = numerical float value
    VKINT,  // integer constant; ival = numerical integer value
    VKSTR,  // string constant; strval = TString address
    VNONRELOC, // expression has its value in a fixed register; info = result register
    VLOCAL, // local variable; var.ridx = register index; var.vidx = relative index in 'actvar.arr'
    VUPVAL, // upvalue variable; info = index of upvalue in 'upvalues'
    VCONST, // compile-time <const> variable; info = absolute index in 'actvar.arr'
    VINDEXED, // indexed variable; ind.t = table register; ind.idx = key's R index
    VINDEXUP, // indexed upvalue; ind.t = upvalue; ind.idx = key's K index
    VINDEXI, // indexed variable with constant integer; ind.t = table register; ind.idx = key's value
    VINDEXSTR, // indexed variable with literal string; ind.t = table register; ind.idx = key's K index
    VJMP,      // expression is a test/comparison; info = pc of corresponding jump instruction
    VRELOC,    // expression can put result in any register; info = instruction pc
    VCALL,     // expression is a function call; info = instruction pc
    VVARARG,   // vararg expression; info = instruction pc
}

#[derive(Clone)]
pub struct ExpDesc {
    pub kind: ExpKind,
    pub u: ExpUnion,
    pub t: isize, // patch list of 'exit when true'
    pub f: isize, // patch list of 'exit when false'
}

#[derive(Clone, Copy)]
pub union ExpUnion {
    pub info: i32,    // for generic use
    pub ival: i64,    // for VKINT
    pub nval: f64,    // for VKFLT
    pub ind: IndVars, // for indexed variables
    pub var: VarVals, // for local/upvalue variables
}

#[derive(Clone, Copy)]
pub struct IndVars {
    pub t: i16,   // table (register or upvalue)
    pub idx: i16, // index (register or constant)
}

#[derive(Clone, Copy)]
pub struct VarVals {
    pub ridx: i16, // register holding the variable
    pub vidx: u16, // compiler index (in 'actvar.arr' or 'upvalues')
}

impl ExpDesc {
    pub fn new_void() -> Self {
        ExpDesc {
            kind: ExpKind::VVOID,
            u: ExpUnion { info: 0 },
            t: -1,
            f: -1,
        }
    }

    pub fn new_nil() -> Self {
        ExpDesc {
            kind: ExpKind::VNIL,
            u: ExpUnion { info: 0 },
            t: -1,
            f: -1,
        }
    }

    pub fn new_int(val: i64) -> Self {
        ExpDesc {
            kind: ExpKind::VKINT,
            u: ExpUnion { ival: val },
            t: -1,
            f: -1,
        }
    }

    pub fn new_float(val: f64) -> Self {
        ExpDesc {
            kind: ExpKind::VKFLT,
            u: ExpUnion { nval: val },
            t: -1,
            f: -1,
        }
    }

    pub fn new_bool(val: bool) -> Self {
        ExpDesc {
            kind: if val { ExpKind::VTRUE } else { ExpKind::VFALSE },
            u: ExpUnion { info: 0 },
            t: -1,
            f: -1,
        }
    }

    pub fn new_k(info: usize) -> Self {
        ExpDesc {
            kind: ExpKind::VK,
            u: ExpUnion { info: info as i32 },
            t: -1,
            f: -1,
        }
    }

    pub fn new_vkstr(string_id: usize) -> Self {
        ExpDesc {
            kind: ExpKind::VKSTR,
            u: ExpUnion { info: string_id as i32 },
            t: -1,
            f: -1,
        }
    }

    pub fn new_nonreloc(reg: u8) -> Self {
        ExpDesc {
            kind: ExpKind::VNONRELOC,
            u: ExpUnion { info: reg as i32 },
            t: -1,
            f: -1,
        }
    }

    pub fn new_local(ridx: u8, vidx: u16) -> Self {
        ExpDesc {
            kind: ExpKind::VLOCAL,
            u: ExpUnion {
                var: VarVals {
                    ridx: ridx as i16,
                    vidx,
                },
            },
            t: -1,
            f: -1,
        }
    }

    pub fn new_upval(idx: u8) -> Self {
        ExpDesc {
            kind: ExpKind::VUPVAL,
            u: ExpUnion { info: idx as i32 },
            t: -1,
            f: -1,
        }
    }

    pub fn new_indexed(t: u8, idx: u8) -> Self {
        ExpDesc {
            kind: ExpKind::VINDEXED,
            u: ExpUnion {
                ind: IndVars {
                    t: t as i16,
                    idx: idx as i16,
                },
            },
            t: -1,
            f: -1,
        }
    }

    pub fn new_reloc(pc: usize) -> Self {
        ExpDesc {
            kind: ExpKind::VRELOC,
            u: ExpUnion { info: pc as i32 },
            t: -1,
            f: -1,
        }
    }

    pub fn new_call(pc: usize) -> Self {
        ExpDesc {
            kind: ExpKind::VCALL,
            u: ExpUnion { info: pc as i32 },
            t: -1,
            f: -1,
        }
    }

    pub fn has_jumps(&self) -> bool {
        // Port of hasjumps macro from lcode.c:58
        // #define hasjumps(e) ((e)->t != (e)->f)
        self.t != self.f
    }

    pub fn is_const(&self) -> bool {
        matches!(
            self.kind,
            ExpKind::VNIL
                | ExpKind::VTRUE
                | ExpKind::VFALSE
                | ExpKind::VK
                | ExpKind::VKFLT
                | ExpKind::VKINT
                | ExpKind::VKSTR
        )
    }

    pub fn is_numeral(&self) -> bool {
        matches!(self.kind, ExpKind::VKINT | ExpKind::VKFLT) && !self.has_jumps()
    }
}
