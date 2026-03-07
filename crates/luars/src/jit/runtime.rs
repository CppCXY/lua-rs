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

    JITModule::new(JITBuilder::with_isa(
        isa,
        cranelift_module::default_libcall_names(),
    ))
}

/// Run `f` with exclusive access to the thread-local `JITModule`.
pub fn with_module<F, R>(f: F) -> R
where
    F: FnOnce(&mut JITModule) -> R,
{
    JIT_MODULE.with(|cell| f(&mut cell.borrow_mut()))
}
