//! Compiles `rsc/y-coustic-config.rc` into the executable (version info).

fn main() {
    y_core::build::embed_resources("rsc/y-coustic-config.rc");
}
