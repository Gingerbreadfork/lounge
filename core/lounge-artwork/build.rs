use std::env;
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let arm32 = target.starts_with("arm") || target.starts_with("thumb");
    let arm = arm32 || target.starts_with("aarch64");

    println!("cargo:rerun-if-changed=csrc/resize.c");
    let mut resize = cc::Build::new();
    resize.file("csrc/resize.c").opt_level(3);
    // 32-bit ARM targets need NEON asked for; 64-bit ARM always has it.
    if arm32 {
        resize.flag("-mfpu=neon-vfpv4");
    }
    resize.compile("lounge_resize");

    // libpng, from the libpng-src crate's unmodified source, with its NEON files on ARM. The
    // prebuilt configuration header is what libpng's own build would generate.
    println!("cargo:rerun-if-changed=csrc/png.c");
    let src = libpng_src::source_path();
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::copy(src.join("scripts/pnglibconf.h.prebuilt"), out.join("pnglibconf.h")).expect("pnglibconf.h");
    let mut png = cc::Build::new();
    png.include(&src).include(&out).opt_level(3).warnings(false);
    for file in [
        "png.c", "pngerror.c", "pngget.c", "pngmem.c", "pngpread.c", "pngread.c", "pngrio.c", "pngrtran.c", "pngrutil.c", "pngset.c", "pngtrans.c",
        "pngwio.c", "pngwrite.c", "pngwtran.c", "pngwutil.c",
    ] {
        png.file(src.join(file));
    }
    if arm {
        for file in ["arm/arm_init.c", "arm/filter_neon_intrinsics.c", "arm/palette_neon_intrinsics.c"] {
            png.file(src.join(file));
        }
    }
    if arm32 {
        png.flag("-mfpu=neon-vfpv4");
    }
    if let Ok(include) = env::var("DEP_Z_INCLUDE") {
        png.include(include);
    }
    png.file("csrc/png.c");
    png.compile("lounge_png");
    // zlib comes after the archive that needs it: the system's on Android and Linux.
    println!("cargo:rustc-link-lib=z");
}
