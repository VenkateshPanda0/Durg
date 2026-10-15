//! Kernel console: `print!`, `println!` and level-tagged log macros.
//!
//! Everything goes to the serial port. There is no framebuffer console
//! (graphics are a non-goal). The macros are textual and defined before the
//! other modules via `#[macro_use]` in `main.rs`.

use core::fmt::{self, Write};

use crate::arch::serial;

struct Console;

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        serial::write_str(s);
        Ok(())
    }
}

#[doc(hidden)]
pub fn print(args: fmt::Arguments) {
    // The console never reports an error, so there is nothing to handle.
    let _ = Console.write_fmt(args);
}

macro_rules! print {
    ($($arg:tt)*) => {
        $crate::console::print(format_args!($($arg)*))
    };
}

macro_rules! println {
    () => {
        print!("\n")
    };
    ($($arg:tt)*) => {
        print!("{}\n", format_args!($($arg)*))
    };
}

/// Informational message, prefixed `[info]`.
macro_rules! info {
    ($($arg:tt)*) => {
        println!("[info] {}", format_args!($($arg)*))
    };
}

/// Something unexpected that the kernel can continue past.
#[allow(unused_macros)]
macro_rules! warn {
    ($($arg:tt)*) => {
        println!("[warn] {}", format_args!($($arg)*))
    };
}

/// A failure. Usually followed by a panic or an exit.
#[allow(unused_macros)]
macro_rules! error {
    ($($arg:tt)*) => {
        println!("[error] {}", format_args!($($arg)*))
    };
}
