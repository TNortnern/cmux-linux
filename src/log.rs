//! Logging for cmux. `cmux_debug!` prints only when CMUX_DEBUG=1. `cmux_info!` and
//! `cmux_warn!` always print to stderr. Every line gets a `cmux:` prefix.
//! Declared with `#[macro_use]` in main.rs, so the macros work in every module.

use std::sync::OnceLock;

/// True when CMUX_DEBUG=1 was set at startup. The environment is read once.
pub fn debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("CMUX_DEBUG").is_some_and(|v| v == "1"))
}

macro_rules! cmux_debug {
    ($($arg:tt)*) => {
        if $crate::log::debug_enabled() {
            eprintln!("cmux: {}", format_args!($($arg)*));
        }
    };
}

macro_rules! cmux_info {
    ($($arg:tt)*) => {
        eprintln!("cmux: {}", format_args!($($arg)*))
    };
}

macro_rules! cmux_warn {
    ($($arg:tt)*) => {
        eprintln!("cmux: warning: {}", format_args!($($arg)*))
    };
}
