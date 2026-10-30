//! Compiles `rsc/y-snap-config.rc` into the executable (icon + version info).

fn main() {
    y_core::build::embed_resources("rsc/y-snap-config.rc");
}
