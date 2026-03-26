use std::time::Duration;

use crate::LuaValue;
#[cfg(feature = "sandbox")]
use crate::Stdlib;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxConfig {
    pub basic: bool,
    pub math: bool,
    pub string: bool,
    pub table: bool,
    pub utf8: bool,
    pub coroutine: bool,
    pub os: bool,
    pub io: bool,
    pub package: bool,
    pub debug: bool,
    pub allow_require: bool,
    pub allow_load: bool,
    pub allow_loadfile: bool,
    pub allow_dofile: bool,
    pub allow_collectgarbage: bool,
    pub injected_globals: Vec<(String, LuaValue)>,
    pub instruction_limit: Option<u64>,
    pub memory_limit_bytes: Option<isize>,
    pub timeout: Option<Duration>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            basic: true,
            math: true,
            string: true,
            table: true,
            utf8: true,
            coroutine: false,
            os: false,
            io: false,
            package: false,
            debug: false,
            allow_require: false,
            allow_load: false,
            allow_loadfile: false,
            allow_dofile: false,
            allow_collectgarbage: false,
            injected_globals: Vec::new(),
            instruction_limit: None,
            memory_limit_bytes: None,
            timeout: None,
        }
    }
}

#[cfg(feature = "sandbox")]
impl SandboxConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_stdlib(mut self, lib: Stdlib) -> Self {
        match lib {
            Stdlib::Basic => self.basic = true,
            Stdlib::Math => self.math = true,
            Stdlib::String => self.string = true,
            Stdlib::Table => self.table = true,
            Stdlib::Utf8 => self.utf8 = true,
            Stdlib::Coroutine => self.coroutine = true,
            Stdlib::Os => self.os = true,
            Stdlib::Io => self.io = true,
            Stdlib::Package => self.package = true,
            Stdlib::Debug => self.debug = true,
            Stdlib::All => {
                self.basic = true;
                self.math = true;
                self.string = true;
                self.table = true;
                self.utf8 = true;
                self.coroutine = true;
                self.os = true;
                self.io = true;
                self.package = true;
                self.debug = true;
            }
        }
        self
    }

    pub fn allow_loading(mut self) -> Self {
        self.allow_load = true;
        self.allow_loadfile = true;
        self.allow_dofile = true;
        self
    }

    pub fn allow_require(mut self) -> Self {
        self.allow_require = true;
        self
    }

    pub fn allow_collectgarbage(mut self) -> Self {
        self.allow_collectgarbage = true;
        self
    }

    pub fn with_global(mut self, name: impl Into<String>, value: LuaValue) -> Self {
        self.injected_globals.push((name.into(), value));
        self
    }

    pub fn insert_global(&mut self, name: impl Into<String>, value: LuaValue) -> &mut Self {
        self.injected_globals.push((name.into(), value));
        self
    }

    pub fn with_instruction_limit(mut self, limit: u64) -> Self {
        self.instruction_limit = Some(limit);
        self
    }

    pub fn with_memory_limit(mut self, limit_bytes: isize) -> Self {
        self.memory_limit_bytes = Some(limit_bytes);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn runtime_limits(&self) -> Option<SandboxRuntimeLimits> {
        let deadline_nanos = self.timeout.map(|timeout| {
            use crate::platform_time::unix_nanos;

            let timeout_nanos = timeout.as_nanos().min(u64::MAX as u128) as u64;
            unix_nanos().saturating_add(timeout_nanos)
        });

        if self.instruction_limit.is_none()
            && self.memory_limit_bytes.is_none()
            && deadline_nanos.is_none()
        {
            return None;
        }

        Some(SandboxRuntimeLimits {
            remaining_instructions: self.instruction_limit,
            memory_limit_bytes: self.memory_limit_bytes,
            deadline_nanos,
            instructions_until_time_check: SANDBOX_TIMEOUT_CHECK_INTERVAL,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxRuntimeLimits {
    pub remaining_instructions: Option<u64>,
    pub memory_limit_bytes: Option<isize>,
    pub deadline_nanos: Option<u64>,
    pub instructions_until_time_check: u32,
}

pub const SANDBOX_TIMEOUT_CHECK_INTERVAL: u32 = 1024;

pub const SANDBOX_SAFE_BASIC_GLOBALS: &[&str] = &[
    "_VERSION",
    "assert",
    "error",
    "getmetatable",
    "ipairs",
    "next",
    "pairs",
    "pcall",
    "print",
    "rawequal",
    "rawget",
    "rawlen",
    "rawset",
    "select",
    "setmetatable",
    "tonumber",
    "tostring",
    "type",
    "warn",
    "xpcall",
];

pub const SANDBOX_LIB_GLOBALS: &[(Stdlib, &str)] = &[
    (Stdlib::Math, "math"),
    (Stdlib::String, "string"),
    (Stdlib::Table, "table"),
    (Stdlib::Utf8, "utf8"),
    (Stdlib::Coroutine, "coroutine"),
    (Stdlib::Os, "os"),
    (Stdlib::Io, "io"),
    (Stdlib::Package, "package"),
    (Stdlib::Debug, "debug"),
];
