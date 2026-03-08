use cranelift_codegen::settings;
use cranelift_jit::{JITBuilder, JITModule};

thread_local! {
    static JIT_MODULE: std::cell::RefCell<JITModule> =
        std::cell::RefCell::new(create_module());
}

fn create_module() -> JITModule {
    // Use default Cranelift settings — the JIT backend already defaults to
    // generating fast code; no need to set opt_level explicitly.
    let isa = cranelift_native::builder()
        .expect("host ISA not supported by Cranelift")
        .finish(settings::Flags::new(settings::builder()))
        .expect("failed to create Cranelift ISA");

    let mut builder = JITBuilder::with_isa(
        isa,
        cranelift_module::default_libcall_names(),
    );

    // Register C `pow` so traces can compile PowFloat.
    unsafe extern "C" { fn pow(x: f64, y: f64) -> f64; }
    builder.symbol("pow", pow as *const u8);

    JITModule::new(builder)
}

/// Run `f` with exclusive access to the thread-local `JITModule`.
pub fn with_module<F, R>(f: F) -> R
where
    F: FnOnce(&mut JITModule) -> R,
{
    JIT_MODULE.with(|cell| f(&mut cell.borrow_mut()))
}
