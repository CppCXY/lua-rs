/// Expression descriptor - tracks expression evaluation state
/// Mirrors Lua's expdesc structure for delayed code generation
/// This allows optimizations like register reuse and constant folding

/// Expression kind - determines how the expression value is represented
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(unused)]
pub enum ExpKind {
    /// No value (void expression)
    VVoid,
    /// Nil constant
    VNil,
    /// True constant
    VTrue,
    /// False constant
    VFalse,
    /// Constant in constant table (info = constant index)
    VK,
    /// Float constant (nval = float value)
    VKFlt,
    /// Integer constant (ival = integer value)
    VKInt,
    /// String constant (strval = string index in constant table)
    VKStr,
    /// Expression has value in a fixed register (info = register)
    VNonReloc,
    /// Local variable (info = register, vidx = local index)
    VLocal,
    /// Upvalue variable (info = upvalue index)
    VUpval,
    /// Indexed variable (ind.t = table reg, ind.idx = key reg)
    VIndexed,
    /// Indexed upvalue (ind.t = upvalue, ind.idx = key constant)
    VIndexUp,
    /// Indexed with constant integer (ind.t = table reg, ind.idx = int value)
    VIndexI,
    /// Indexed with literal string (ind.t = table reg, ind.idx = string constant)
    VIndexStr,
    /// Expression is a test/comparison (info = jump instruction pc)
    VJmp,
    /// Expression can put result in any register (info = instruction pc)
    VReloc,
    /// Expression is a function call (info = instruction pc)
    VCall,
    /// Vararg expression (info = instruction pc)
    VVararg,
}

/// Index information for indexed expressions
#[derive(Debug, Clone, Copy)]
pub struct IndexInfo {
    pub t: u32,   // Table register or upvalue
    pub idx: u32, // Key register or constant index
}

/// Local variable information
#[derive(Debug, Clone, Copy)]
pub struct VarInfo {
    pub ridx: u32, // Register index
    #[allow(unused)]
    pub vidx: usize, // Variable index in locals array
}

/// Expression descriptor
#[derive(Debug, Clone)]
pub struct ExpDesc {
    pub kind: ExpKind,
    /// Generic info field - meaning depends on kind
    pub info: u32,
    /// Integer value (for VKInt)
    pub ival: i64,
    /// Float value (for VKFlt)
    pub nval: f64,
    /// Index information (for VIndexed, VIndexUp, VIndexI, VIndexStr)
    pub ind: IndexInfo,
    /// Variable information (for VLocal)
    pub var: VarInfo,
    /// Patch list for 'exit when true' jumps
    pub t: i32,
    /// Patch list for 'exit when false' jumps
    pub f: i32,
}

impl ExpDesc {
    /// Create a new void expression
    #[allow(dead_code)]
    pub fn new_void() -> Self {
        ExpDesc {
            kind: ExpKind::VVoid,
            info: 0,
            ival: 0,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: 0, vidx: 0 },
            t: -1,
            f: -1,
        }
    }

    /// Create expression in a specific register
    pub fn new_nonreloc(reg: u32) -> Self {
        ExpDesc {
            kind: ExpKind::VNonReloc,
            info: reg,
            ival: 0,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: 0, vidx: 0 },
            t: -1,
            f: -1,
        }
    }

    /// Create local variable expression
    #[allow(dead_code)]
    pub fn new_local(reg: u32, vidx: usize) -> Self {
        ExpDesc {
            kind: ExpKind::VLocal,
            info: 0,
            ival: 0,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: reg, vidx },
            t: -1,
            f: -1,
        }
    }

    /// Create integer constant expression
    pub fn new_int(val: i64) -> Self {
        ExpDesc {
            kind: ExpKind::VKInt,
            info: 0,
            ival: val,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: 0, vidx: 0 },
            t: -1,
            f: -1,
        }
    }

    /// Create float constant expression
    pub fn new_float(val: f64) -> Self {
        ExpDesc {
            kind: ExpKind::VKFlt,
            info: 0,
            ival: 0,
            nval: val,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: 0, vidx: 0 },
            t: -1,
            f: -1,
        }
    }

    /// Create constant table expression
    pub fn new_k(const_idx: u32) -> Self {
        ExpDesc {
            kind: ExpKind::VK,
            info: const_idx,
            ival: 0,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: 0, vidx: 0 },
            t: -1,
            f: -1,
        }
    }

    /// Create string constant expression (对齐luac VKStr)
    pub fn new_kstr(str_idx: u32) -> Self {
        ExpDesc {
            kind: ExpKind::VKStr,
            info: str_idx,
            ival: 0,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: 0, vidx: 0 },
            t: -1,
            f: -1,
        }
    }

    /// Create nil expression
    pub fn new_nil() -> Self {
        ExpDesc {
            kind: ExpKind::VNil,
            info: 0,
            ival: 0,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: 0, vidx: 0 },
            t: -1,
            f: -1,
        }
    }

    /// Create true expression
    pub fn new_true() -> Self {
        ExpDesc {
            kind: ExpKind::VTrue,
            info: 0,
            ival: 0,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: 0, vidx: 0 },
            t: -1,
            f: -1,
        }
    }

    /// Create false expression
    pub fn new_false() -> Self {
        ExpDesc {
            kind: ExpKind::VFalse,
            info: 0,
            ival: 0,
            nval: 0.0,
            ind: IndexInfo { t: 0, idx: 0 },
            var: VarInfo { ridx: 0, vidx: 0 },
            t: -1,
            f: -1,
        }
    }

    /// Check if expression is a variable
    #[allow(dead_code)]
    pub fn is_var(&self) -> bool {
        matches!(
            self.kind,
            ExpKind::VLocal
                | ExpKind::VUpval
                | ExpKind::VIndexed
                | ExpKind::VIndexUp
                | ExpKind::VIndexI
                | ExpKind::VIndexStr
        )
    }

    /// Check if expression has multiple returns
    #[allow(dead_code)]
    pub fn has_multret(&self) -> bool {
        matches!(self.kind, ExpKind::VCall | ExpKind::VVararg)
    }

    /// Get the register number if expression is in a register
    pub fn get_register(&self) -> Option<u32> {
        match self.kind {
            ExpKind::VNonReloc => Some(self.info),
            ExpKind::VLocal => Some(self.var.ridx),
            _ => None,
        }
    }
}

/// Check if expression can be used as RK operand (register or constant)
#[allow(dead_code)]
pub fn is_rk(e: &ExpDesc) -> bool {
    matches!(
        e.kind,
        ExpKind::VK | ExpKind::VKInt | ExpKind::VKFlt | ExpKind::VNonReloc | ExpKind::VLocal
    )
}

/// Check if expression is a constant
#[allow(dead_code)]
pub fn is_const(e: &ExpDesc) -> bool {
    matches!(
        e.kind,
        ExpKind::VNil
            | ExpKind::VTrue
            | ExpKind::VFalse
            | ExpKind::VK
            | ExpKind::VKInt
            | ExpKind::VKFlt
            | ExpKind::VKStr
    )
}

/// Check if expression is a numeric constant
#[allow(dead_code)]
pub fn is_numeral(e: &ExpDesc) -> bool {
    matches!(e.kind, ExpKind::VKInt | ExpKind::VKFlt)
}
