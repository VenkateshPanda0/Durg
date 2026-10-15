//! Passes the linker script to the linker.
//!
//! Done here instead of in `.cargo/config.toml` so the flag applies only to the
//! kernel crate and not to host-side tools in the same workspace.

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    println!("cargo:rustc-link-arg=-T{dir}/linker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
}
