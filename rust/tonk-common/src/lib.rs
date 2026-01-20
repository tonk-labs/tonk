#![warn(missing_docs)]
//! Cross-platform utilities for the Tonk project.

/// Cross-platform logging macro that uses `console.log` on web and `println!` on native.
///
/// # Examples
///
/// ```
/// use tonk_common::log;
///
/// log!("Hello, world!");
/// log!("Value: {}", 42);
/// log!("Multiple values: {} and {}", "foo", "bar");
/// ```
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            web_sys::console::log_1(&format!($($arg)*).into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            println!($($arg)*);
        }
    }};
}

mod exclusive;
pub use exclusive::*;
