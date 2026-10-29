fn main() {
    println!("cargo:rerun-if-changed=csrc/resize.c");
    let mut build = cc::Build::new();
    build.file("csrc/resize.c").opt_level(3);
    // 32-bit ARM targets need NEON asked for; 64-bit ARM always has it.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.starts_with("arm") || target.starts_with("thumb") {
        build.flag("-mfpu=neon-vfpv4");
    }
    build.compile("lounge_resize");
}
