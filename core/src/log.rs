//! A tiny file logger.
//!
//! Every Ypo-Lite application is `windows_subsystem = "windows"` — there is
//! no console, and one attached via `cargo run` doesn't reliably work
//! either: confirmed in this repo's own development, `eprintln!` from
//! `y-pop`'s `main` printed nothing at all when run from a Git Bash/MinTTY
//! terminal (MinTTY emulates a terminal over pipes rather than allocating a
//! real Win32 console the way `cmd.exe`/`conhost.exe` does, and a
//! GUI-subsystem process's stdio does not attach to it the same way). A
//! file next to wherever the application already keeps its own state is the
//! one sink that reliably works everywhere, terminal or none.
//!
//! Pure Rust, no Win32: `std::fs` is all this needs.
//!
//! ```no_run
//! y_core::log::init(std::path::Path::new("y-pop.log")).ok();
//! y_core::log::line("started");
//! ```

use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static FILE: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

/// Opens (creating, or appending to an existing) the log file at `path`,
/// creating its parent directory if needed. Idempotent: once a file is
/// open, a later call — even with a different path — is a no-op, since the
/// point is one file for the process's whole life, not the most recent
/// caller's opinion of where it should be.
pub fn init(path: &Path) -> std::io::Result<()> {
    if FILE.get().is_some() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;

    START.get_or_init(Instant::now);
    let _ = FILE.set(Mutex::new(file));
    Ok(())
}

/// Appends one line, prefixed with seconds since [`init`] and flushed
/// immediately — so a line written just before a crash is still on disk.
/// A silent no-op if [`init`] was never called, failed, or another thread
/// holds the file lock poisoned: logging is diagnostic, never load-bearing,
/// so nothing here is a reason to fail or panic whatever called it.
pub fn line(message: impl fmt::Display) {
    let Some(file) = FILE.get() else { return };
    let Ok(mut file) = file.lock() else { return };

    let elapsed = START
        .get()
        .map_or(0.0, |start| start.elapsed().as_secs_f64());
    let _ = writeln!(file, "[{elapsed:8.3}] {message}");
    let _ = file.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_back_a_line() {
        // A process-global OnceLock means this can only be exercised once
        // per test binary — the file is scoped to this test's own temp
        // path, and every other test in this file must avoid calling
        // `init`/`line` to not observe (or fight over) the same globals.
        let path = std::env::temp_dir().join("y-core-log-test.log");
        let _ = std::fs::remove_file(&path);

        init(&path).expect("open log file");
        line("hello");
        line(format_args!("count={}", 3));

        let contents = std::fs::read_to_string(&path).expect("read log file");
        assert!(contents.contains("hello"));
        assert!(contents.contains("count=3"));

        let _ = std::fs::remove_file(&path);
    }
}
