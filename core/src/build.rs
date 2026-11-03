//! Helper for the applications' `build.rs`, behind the `build` feature.
//!
//! ```no_run
//! // In `main` of y-snap/build.rs:
//! y_core::build::embed_resources("rsc/y-snap.rc");
//! ```
//!
//! `embed-resource` picks whichever resource compiler the active toolchain
//! provides (`rc.exe` from the Windows SDK, `llvm-rc`, or `windres`), so the
//! same resource script works for every Windows host.
//!
//! A hand-written `.rc` is preferred over the `winres`/`winresource` crates,
//! which generate the script for you: the moment an application adds a menu, a
//! dialog or a string table, it wants to own that file.
//!
//! # The version
//!
//! The script does not carry the version. It writes two tokens, and the
//! application's `Cargo.toml` version (the workspace's, for every app here) is
//! put in their place at build time:
//!
//! | Token | Becomes (version `0.2.0`) | For |
//! | --- | --- | --- |
//! | `@VERSION_COMMAS@` | `0,2,0,0` | `FILEVERSION`, `PRODUCTVERSION` |
//! | `@VERSION@` | `0.2.0.0` | the `FileVersion` and `ProductVersion` strings |
//!
//! Only major, minor and patch are used: a pre-release suffix does not fit a
//! Windows version number.

use std::path::{Path, PathBuf};

const VERSION_COMMAS: &str = "@VERSION_COMMAS@";
const VERSION_DOTS: &str = "@VERSION@";

/// Compiles `script` into the executable, with the version of the crate being
/// built put in (see the module documentation).
///
/// Also watches the files the script includes — `embed-resource` watches the
/// `.rc` itself, but not the icon or manifest it names, so editing an icon
/// would otherwise not trigger a rebuild.
pub fn embed_resources(script: &str) {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={script}");
    for included in included_files(script) {
        println!("cargo:rerun-if-changed={included}");
    }

    // Keep `cargo check` usable when cross-compiling away from Windows.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let generated = generate(script);

    // `manifest_required` turns a missing resource compiler into a hard error
    // instead of silently producing a binary without icon or manifest.
    embed_resource::compile(&generated, embed_resource::NONE)
        .manifest_required()
        .expect("failed to compile Windows resources");
}

/// The file names quoted in the script, resolved next to it: the icon, the
/// manifest, anything else it pulls in.
fn included_files(script: &str) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(script) else {
        return Vec::new();
    };
    let directory = Path::new(script).parent().unwrap_or(Path::new("."));

    text.lines()
        .filter_map(|line| {
            let quoted = line.split('"').nth(1)?;
            let path = directory.join(quoted);
            path.is_file().then(|| path.display().to_string())
        })
        .collect()
}

/// Writes the script to `OUT_DIR` with the version filled in, and returns its
/// path. The script's own directory is no longer next to it, so the files it
/// names (icon, manifest) are rewritten as absolute paths.
fn generate(script: &str) -> PathBuf {
    let text = std::fs::read_to_string(script)
        .unwrap_or_else(|error| panic!("cannot read {script}: {error}"));
    assert!(
        text.contains(VERSION_COMMAS) && text.contains(VERSION_DOTS),
        "{script} must use {VERSION_COMMAS} (FILEVERSION, PRODUCTVERSION) and {VERSION_DOTS} \
         (FileVersion, ProductVersion) so that its version follows Cargo.toml"
    );

    // Set by Cargo for the build script of the application being built.
    let number = |name: &str| {
        std::env::var(name)
            .unwrap_or_else(|_| panic!("{name} is not set"))
            .parse::<u16>()
            .unwrap_or_else(|_| panic!("{name} is not a number a Windows version can hold"))
    };
    let (major, minor, patch) = (
        number("CARGO_PKG_VERSION_MAJOR"),
        number("CARGO_PKG_VERSION_MINOR"),
        number("CARGO_PKG_VERSION_PATCH"),
    );
    let commas = format!("{major},{minor},{patch},0");
    let dots = format!("{major}.{minor}.{patch}.0");

    let directory = Path::new(script).parent().unwrap_or(Path::new("."));
    let text = text
        .lines()
        .map(|line| {
            let line = line
                .replace(VERSION_COMMAS, &commas)
                .replace(VERSION_DOTS, &dots);
            absolute_include(&line, directory).unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let name = Path::new(script).file_name().expect("script file name");
    let generated = Path::new(&out_dir).join(name);
    std::fs::write(&generated, text)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", generated.display()));
    generated
}

/// `line` with its first quoted file name made absolute, or `None` if that
/// string does not name a file next to the script. Forward slashes, because a
/// backslash starts an escape inside a resource script's string.
fn absolute_include(line: &str, directory: &Path) -> Option<String> {
    let quoted = line.split('"').nth(1)?;
    let path = directory.join(quoted);
    if !path.is_file() {
        return None;
    }
    let absolute = std::path::absolute(path).ok()?;
    let absolute = absolute.to_string_lossy().replace('\\', "/");
    Some(line.replacen(&format!("\"{quoted}\""), &format!("\"{absolute}\""), 1))
}
