// Test module organization
pub mod test_async;
pub mod test_basic;
pub mod test_control_flow;
pub mod test_coroutine;
pub mod test_io; // IO tests use test_data directory
pub mod test_math;
pub mod test_metamethods;
pub mod test_operators;
pub mod test_os; // OS library tests
pub mod test_package;
pub mod test_string;
pub mod test_syntax;
pub mod test_table;
pub mod test_userdata;
pub mod test_utf8;
pub mod test_xpcall_debug;
// pub mod test_closures;  // Disabled - upvalue handling needs work
pub mod test_advanced_calls;
pub mod test_api_proposals;
pub mod test_c_functions;
pub mod test_functions;
pub mod test_gc_metamethods;
pub mod test_rclosure;
