//! Compiles `rsc/win-rust-min.rc` into the executable.
//!
//! `embed-resource` picks the right tool for the active toolchain (`rc.exe`
//! from the Windows SDK, `llvm-rc`, or `windres` for the GNU target), so the
//! same resource script works for every Windows host.

const RESOURCE_SCRIPT: &str = "rsc/win-rust-min.rc";
const MANIFEST: &str = "rsc/win-rust-min.exe.manifest";
const ICON: &str = "rsc/usb.minimal.ico";

fn main() {
    // `embed-resource` watches the .rc itself, but not the files it includes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={RESOURCE_SCRIPT}");
    println!("cargo:rerun-if-changed={MANIFEST}");
    println!("cargo:rerun-if-changed={ICON}");

    // Keep `cargo check` usable when cross-compiling away from Windows.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    assert_version_in_sync();

    // `manifest_required` turns a missing resource compiler into a hard error
    // instead of silently producing a binary without icon or manifest.
    embed_resource::compile(RESOURCE_SCRIPT, embed_resource::NONE)
        .manifest_required()
        .expect("failed to compile Windows resources");
}

/// Fails the build when `FILEVERSION` in the resource script drifts away from
/// the version in `Cargo.toml` — the two are edited in different files and
/// otherwise diverge silently.
fn assert_version_in_sync() {
    let script = std::fs::read_to_string(RESOURCE_SCRIPT)
        .unwrap_or_else(|e| panic!("cannot read {RESOURCE_SCRIPT}: {e}"));

    let declared = script
        .lines()
        .find_map(|line| line.trim().strip_prefix("FILEVERSION"))
        .map(|value| {
            value
                .split(',')
                .take(3)
                .map(|part| part.trim().to_owned())
                .collect::<Vec<_>>()
                .join(".")
        })
        .unwrap_or_else(|| panic!("no FILEVERSION found in {RESOURCE_SCRIPT}"));

    let expected = format!(
        "{}.{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
    );

    assert_eq!(
        declared, expected,
        "version mismatch: {RESOURCE_SCRIPT} declares {declared}, Cargo.toml declares {expected} \
         (update FILEVERSION, PRODUCTVERSION and the version strings in the .rc)"
    );
}
