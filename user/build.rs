//! Passes the user linker script to the linker, for these programs only (not the
//! kernel or the host tools in the same workspace).

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    println!("cargo:rustc-link-arg-bins=-T{dir}/user.ld");
    // Segments are page-aligned and the loader maps whole pages.
    println!("cargo:rustc-link-arg-bins=-zmax-page-size=4096");
    println!("cargo:rerun-if-changed=user.ld");
}
