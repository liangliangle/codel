//! Startup instrumentation stubs. The Chrome-trace instrumentation backend
//! was removed with the telemetry system; these minimal types keep call sites
//! compiling without any runtime effect.

pub const TARGET: &str = "instrumentation";

pub struct InstrumentationTimer {
    _name: &'static str,
}

impl InstrumentationTimer {
    pub fn new(name: &'static str) -> Self {
        Self { _name: name }
    }

    pub fn with_field(&mut self, _key: &str, _value: impl std::fmt::Display) -> &mut Self {
        self
    }
}

#[macro_export]
macro_rules! instrumentation_timer {
    ($name:expr) => {
        $crate::instrumentation::InstrumentationTimer::new($name)
    };
}

pub fn timer(name: &'static str) -> InstrumentationTimer {
    InstrumentationTimer::new(name)
}

pub fn finalize_and_exit(code: i32) -> ! {
    std::process::exit(code)
}
