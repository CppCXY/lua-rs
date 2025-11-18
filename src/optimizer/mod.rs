// SSA-based bytecode optimizer
// Performs optimization at bytecode level using SSA analysis

mod ssa_builder;
mod optimizations;
mod simple_optimizer;

pub use ssa_builder::{SSABuilder, SSAValue, SSAInstruction};
pub use simple_optimizer::optimize_constants;

/// Optimized bytecode function
#[derive(Debug, Clone)]
pub struct OptimizedFunction {
    pub instructions: Vec<u32>,
    pub constants: Vec<crate::lua_value::LuaValue>,
    pub max_stack: usize,
}

/// Optimization configuration
#[derive(Debug, Clone)]
pub struct OptConfig {
    pub constant_folding: bool,
    pub dead_code_elimination: bool,
    pub copy_propagation: bool,
    pub algebraic_simplification: bool,
    pub loop_invariant_motion: bool,
    pub strength_reduction: bool,
}

impl Default for OptConfig {
    fn default() -> Self {
        Self {
            constant_folding: true,
            dead_code_elimination: true,
            copy_propagation: true,
            algebraic_simplification: true,
            loop_invariant_motion: true,
            strength_reduction: true,
        }
    }
}

/// Main optimization entry point
pub fn optimize_bytecode(
    instructions: &[u32],
    constants: &[crate::lua_value::LuaValue],
    config: &OptConfig,
) -> OptimizedFunction {
    // Build SSA form
    let ssa = SSABuilder::from_bytecode(instructions, constants);
    
    // Apply optimizations
    let optimized_ssa = optimizations::apply_optimizations(ssa, config);
    
    // Convert back to bytecode
    optimizations::ssa_to_bytecode(optimized_ssa, constants)
}
