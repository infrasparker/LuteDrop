// LuteDrop build script.
//
// Dependency option (A): vcpkg static. We compile the miniaudio + ma_libopus
// C shim with the `cc` crate and link the xiph FOSS stack (opusfile/opus/ogg)
// statically from a vcpkg `x64-windows-static` install.
//
// Option (B) -- fully vendored ogg/opus/opusfile sources compiled with `cc`,
// no vcpkg/CMake -- was the hermetic ideal but NOT pursued: cc-compiling
// libopus 1.5.2 from raw sources requires hand-authoring the autotools/CMake
// `config.h` plus wiring ~100 source files across celt/silk/src with the right
// fixed/float + SIMD macros for MSVC. That is intractable within a reasonable
// effort box, which is exactly the documented fallback trigger. Option (A) is
// the proven path from the 2026-05-30 spike (vcpkg already installed at
// C:\Users\Infrasparker\vcpkg, opusfile:x64-windows-static built), so we use it.
//
// To recreate the vcpkg install:
//   git clone --depth 1 https://github.com/microsoft/vcpkg C:\Users\Infrasparker\vcpkg
//   C:\Users\Infrasparker\vcpkg\bootstrap-vcpkg.bat -disableMetrics
//   C:\Users\Infrasparker\vcpkg\vcpkg install opusfile:x64-windows-static
// (vcpkg uses CMake internally; CMake stays a prerequisite under option A.)

use std::path::PathBuf;

fn main() {
    build_audio_shim();
    tauri_build::build();
}

fn build_audio_shim() {
    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");

    // Resolve vcpkg root: prefer the VCPKG_ROOT env var, fall back to %USERPROFILE%\vcpkg.
    let vcpkg_root = if let Ok(root) = std::env::var("VCPKG_ROOT") {
        PathBuf::from(root)
    } else {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .expect("Neither VCPKG_ROOT nor USERPROFILE/HOME is set; cannot locate vcpkg");
        PathBuf::from(home).join("vcpkg")
    };

    // vcpkg x64-windows-static triplet install root.
    let vcpkg_installed = vcpkg_root.join("installed").join("x64-windows-static");

    if !vcpkg_installed.exists() {
        panic!(
            "vcpkg x64-windows-static triplet not found at `{}`.\n\
             Run: vcpkg install opusfile:x64-windows-static\n\
             If vcpkg is not at the default location (~\\vcpkg), set the VCPKG_ROOT \
             environment variable to its root directory.\n\
             See the README for full setup instructions.",
            vcpkg_installed.display()
        );
    }

    let include = vcpkg_installed.join("include");
    let lib = vcpkg_installed.join("lib");

    // Compile the C shim: miniaudio impl + ma_libopus backend + ld_decode_file.
    cc::Build::new()
        .file("vendor/shim.c")
        .include("vendor") // miniaudio.h, miniaudio_libopus.h/.c, shim.h
        .include(&include) // ogg/ headers
        .include(include.join("opus")) // opusfile.h + opus headers live here
        // Match vcpkg x64-windows-static: static CRT (/MT).
        .static_crt(true)
        .compile("ld_audio_shim");

    // Link the xiph FOSS stack (order matters for static link: opusfile -> opus -> ogg).
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=opusfile");
    println!("cargo:rustc-link-lib=static=opus");
    println!("cargo:rustc-link-lib=static=ogg");

    println!("cargo:rerun-if-changed=vendor/shim.c");
    println!("cargo:rerun-if-changed=vendor/shim.h");
    println!("cargo:rerun-if-changed=vendor/miniaudio_libopus.c");
    println!("cargo:rerun-if-changed=vendor/miniaudio_libopus.h");
}
