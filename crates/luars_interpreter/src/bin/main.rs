use luars_interpreter::run_interpreter;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() {
    run_interpreter()
}
