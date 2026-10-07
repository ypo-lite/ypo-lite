//! Compiles `rsc/y-pop.rc` into the executable (icon + version info).

fn main() {
    y_core::build::embed_resources("rsc/y-pop.rc");
}
