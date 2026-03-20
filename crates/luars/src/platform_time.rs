#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PlatformInstant {
    inner: std::time::Instant,
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct PlatformInstant {
    start_ms: f64,
}

impl PlatformInstant {
    #[inline]
    pub(crate) fn now() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                inner: std::time::Instant::now(),
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            Self {
                start_ms: js_sys::Date::now(),
            }
        }
    }

    #[inline]
    pub(crate) fn elapsed_secs_f64(&self) -> f64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.inner.elapsed().as_secs_f64()
        }

        #[cfg(target_arch = "wasm32")]
        {
            (js_sys::Date::now() - self.start_ms) / 1000.0
        }
    }
}

#[inline]
pub(crate) fn unix_nanos() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }

    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() * 1_000_000.0) as u64
    }
}

#[inline]
pub(crate) fn unix_secs() -> u64 {
    unix_nanos() / 1_000_000_000
}