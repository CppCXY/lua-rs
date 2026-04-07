mod compile;
mod model;
#[cfg(feature = "jit")]
mod native;

pub(crate) use model::*;
#[cfg(feature = "jit")]
pub(crate) use native::NativeTraceBackend;
#[cfg(not(feature = "jit"))]
pub(crate) type NativeTraceBackend = NullTraceBackend;

#[cfg(test)]
mod test;
