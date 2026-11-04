//! Compiles `rsc/y-coustic.rc` into the executable (manifest + version info).

fn main() {
    y_core::build::embed_resources("rsc/y-coustic.rc");
}
