fn main() {
    println!("cargo:rerun-if-changed=src/platform/apple/locale.m");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        println!("cargo:rerun-if-changed=src/platform/ios/mobile.m");
        cc::Build::new()
            .file("src/platform/ios/mobile.m")
            .file("src/platform/apple/locale.m")
            .flag("-fobjc-arc")
            .compile("nabi_mobile");
        for framework in [
            "UIKit",
            "Foundation",
            "UniformTypeIdentifiers",
            "CoreText",
            "ImageIO",
            "CoreGraphics",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rerun-if-changed=src/platform/android/mobile.c");
        cc::Build::new()
            .file("src/platform/android/mobile.c")
            .compile("nabi_mobile");
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_icon();
    }
    if std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() != Some(std::ffi::OsStr::new("macos")) {
        return;
    }
    let bridge = "src/platform/macos/open_documents.m";
    println!("cargo:rerun-if-changed={bridge}");
    println!("cargo:rerun-if-changed=src/platform/macos/trash.m");
    println!("cargo:rerun-if-changed=src/platform/macos/fullscreen.m");
    println!("cargo:rerun-if-changed=src/platform/macos/display_sleep.m");
    cc::Build::new()
        .file(bridge)
        .file("src/platform/apple/locale.m")
        .file("src/platform/macos/fullscreen.m")
        .file("src/platform/macos/trash.m")
        .file("src/platform/macos/display_sleep.m")
        .flag("-fobjc-arc")
        .compile("nabi_open_documents");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=UniformTypeIdentifiers");
    println!("cargo:rustc-link-lib=framework=ImageIO");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
    println!("cargo:rustc-link-lib=framework=IOKit");
}

fn windows_icon() {
    use std::{env, path::PathBuf, process::Command};
    let target = env::var("TARGET").unwrap();
    let icon = "assets/icons/nabi-icon.ico";
    println!("cargo:rerun-if-changed={icon}");
    println!("cargo:rerun-if-env-changed=RC");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let source = out.join("nabi-icon.rc");
    let absolute = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join(icon);
    std::fs::write(
        &source,
        format!(
            "1 ICON \"{}\"\n",
            absolute.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    let msvc = target.ends_with("msvc");
    let output = out.join(if msvc { "nabi-icon.res" } else { "nabi-icon.o" });
    let mut command = if let Some(rc) = env::var_os("RC") {
        Command::new(rc)
    } else if msvc {
        cc::windows_registry::find_tool(&target, "rc.exe")
            .expect("Windows SDK resource compiler required (or set RC)")
            .to_command()
    } else {
        Command::new(if target.starts_with("x86_64") {
            "x86_64-w64-mingw32-windres"
        } else {
            "windres"
        })
    };
    if msvc {
        command.arg("/nologo").arg("/fo").arg(&output).arg(&source);
    } else {
        command
            .arg("-i")
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .args(["-O", "coff"]);
    }
    assert!(
        command.status().expect("run resource compiler").success(),
        "icon resource compilation failed"
    );
    println!("cargo:rustc-link-arg-bin=nabi-view={}", output.display());
}
