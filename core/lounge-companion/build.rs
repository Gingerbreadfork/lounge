// The web app is embedded at compile time; a fresh `companion/dist` must recompile this crate.
fn main() {
    println!("cargo:rerun-if-changed=../../companion/dist");
    println!("cargo:rerun-if-changed=build.rs");
}
