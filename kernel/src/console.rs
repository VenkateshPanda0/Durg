//! Kernel console: `print!`, `println!` and level-tagged log macros.
//!
//! Everything goes to the serial port. There is no framebuffer console
//! (graphics are a non-goal). The macros are textual and defined before the
//! other modules via `#[macro_use]` in `main.rs`.

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::serial;
use crate::arch::x86_64::IrqLock;

/// Held while a message is written, so two threads' lines do not interleave
/// and a timer interrupt cannot switch threads halfway through a line.
static CONSOLE: IrqLock<()> = IrqLock::new(());

/// Set once the kernel is dying. A panic or fatal exception may happen while
/// this very thread holds the console lock (a panic inside a `Display` impl
/// called by `println!`); waiting for it would lose the message. After this,
/// output skips the lock: the kernel is about to exit, so there is nothing left
/// to protect.
static PANIC_MODE: AtomicBool = AtomicBool::new(false);

/// Lets the dying kernel print without taking the console lock.
pub fn enter_panic_mode() {
    PANIC_MODE.store(true, Ordering::SeqCst);
}

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
    if PANIC_MODE.load(Ordering::SeqCst) {
        let _ = Console.write_fmt(args);
    } else {
        let _guard = CONSOLE.lock();
        let _ = Console.write_fmt(args);
    }
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
