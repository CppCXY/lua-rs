//! EmmyLua debugger protocol types.
//!
//! Wire format: each message is two newline-terminated lines:
//!   `<cmd_number>\n<json_body>\n`
//!
//! The receiver skips the first line (cmd header) and dispatches on
//! the `"cmd"` field inside the JSON body.

use serde::{Deserialize, Serialize};

// ============ MessageCMD ============

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MessageCMD {
    Unknown = 0,
    InitReq = 1,
    InitRsp = 2,
    ReadyReq = 3,
    ReadyRsp = 4,
    AddBreakPointReq = 5,
    AddBreakPointRsp = 6,
    RemoveBreakPointReq = 7,
    RemoveBreakPointRsp = 8,
    ActionReq = 9,
    ActionRsp = 10,
    EvalReq = 11,
    EvalRsp = 12,
    BreakNotify = 13,
    AttachedNotify = 14,
    StartHookReq = 15,
    StartHookRsp = 16,
    LogNotify = 17,
}

impl MessageCMD {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::InitReq,
            2 => Self::InitRsp,
            3 => Self::ReadyReq,
            4 => Self::ReadyRsp,
            5 => Self::AddBreakPointReq,
            6 => Self::AddBreakPointRsp,
            7 => Self::RemoveBreakPointReq,
            8 => Self::RemoveBreakPointRsp,
            9 => Self::ActionReq,
            10 => Self::ActionRsp,
            11 => Self::EvalReq,
            12 => Self::EvalRsp,
            13 => Self::BreakNotify,
            14 => Self::AttachedNotify,
            15 => Self::StartHookReq,
            16 => Self::StartHookRsp,
            17 => Self::LogNotify,
            _ => Self::Unknown,
        }
    }
}

// ============ DebugAction ============

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum DebugAction {
    None = -1,
    Break = 0,
    Continue = 1,
    StepOver = 2,
    StepIn = 3,
    StepOut = 4,
    Stop = 5,
}

impl DebugAction {
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => Self::Break,
            1 => Self::Continue,
            2 => Self::StepOver,
            3 => Self::StepIn,
            4 => Self::StepOut,
            5 => Self::Stop,
            _ => Self::None,
        }
    }
}

// ============ IDE → Debugger messages ============

#[derive(Debug, Deserialize)]
pub struct InitReqBody {
    pub cmd: i32,
    #[serde(default)]
    pub ext: Vec<String>,
    #[serde(rename = "emmyHelper", default)]
    pub emmy_helper: String,
}

#[derive(Debug, Deserialize)]
pub struct ReadyReqBody {
    pub cmd: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BreakPointProto {
    pub file: String,
    pub line: i32,
    #[serde(default)]
    pub condition: String,
    #[serde(rename = "hitCondition", default)]
    pub hit_condition: String,
    #[serde(rename = "logMessage", default)]
    pub log_message: String,
}

#[derive(Debug, Deserialize)]
pub struct AddBreakPointReqBody {
    pub cmd: i32,
    #[serde(default)]
    pub clear: bool,
    #[serde(rename = "breakPoints", default)]
    pub break_points: Vec<BreakPointProto>,
}

#[derive(Debug, Deserialize)]
pub struct RemoveBreakPointReqBody {
    pub cmd: i32,
    #[serde(rename = "breakPoints", default)]
    pub break_points: Vec<BreakPointProto>,
}

#[derive(Debug, Deserialize)]
pub struct ActionReqBody {
    pub cmd: i32,
    pub action: i32,
}

#[derive(Debug, Deserialize)]
pub struct EvalReqBody {
    pub cmd: i32,
    pub seq: i32,
    #[serde(default)]
    pub expr: String,
    #[serde(rename = "stackLevel", default)]
    pub stack_level: i32,
    #[serde(default)]
    pub depth: i32,
    #[serde(rename = "cacheId", default)]
    pub cache_id: i32,
    #[serde(default)]
    pub value: String,
    #[serde(rename = "setValue", default)]
    pub set_value: bool,
}

// ============ Debugger → IDE messages ============

/// Variable information sent in BreakNotify and EvalRsp.
#[derive(Debug, Clone, Serialize)]
pub struct VariableProto {
    pub name: String,
    #[serde(rename = "nameType")]
    pub name_type: i32,
    pub value: String,
    #[serde(rename = "valueType")]
    pub value_type: i32,
    #[serde(rename = "valueTypeName")]
    pub value_type_name: String,
    #[serde(rename = "cacheId")]
    pub cache_id: i32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<VariableProto>,
}

/// Stack frame information sent in BreakNotify.
#[derive(Debug, Clone, Serialize)]
pub struct StackProto {
    pub file: String,
    #[serde(rename = "functionName")]
    pub function_name: String,
    pub line: i32,
    pub level: i32,
    #[serde(rename = "localVariables")]
    pub local_variables: Vec<VariableProto>,
    #[serde(rename = "upvalueVariables")]
    pub upvalue_variables: Vec<VariableProto>,
}

#[derive(Debug, Serialize)]
pub struct BreakNotifyBody {
    pub cmd: i32, // 13
    pub stacks: Vec<StackProto>,
}

#[derive(Debug, Serialize)]
pub struct EvalRspBody {
    pub seq: i32,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<VariableProto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AttachedNotifyBody {
    pub state: i64,
}

#[derive(Debug, Serialize)]
pub struct LogNotifyBody {
    #[serde(rename = "type")]
    pub log_type: i32,
    pub message: String,
}

// ============ Log type ============

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
#[repr(i32)]
pub enum LogType {
    Info = 0,
    Warning = 1,
    Error = 2,
}
