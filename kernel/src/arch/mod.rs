//! Architecture layer. Everything that touches CPU instructions lives here, so
//! `unsafe` code stays in a small, auditable place (see ADR-0001).

pub mod serial;
pub mod x86_64;

/// Stops the CPU forever.
pub fn halt() -> ! {
    loop {
        x86_64::hlt();
    }
}
