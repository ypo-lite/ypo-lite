//! Working set trimming.
//!
//! Read this before reaching for it: trimming does **not** free memory. It
//! unmaps pages from the process working set and hands them to the standby
//! list, so Task Manager's "Memory" column drops — but the commit charge is
//! unchanged, and every page that is touched again comes back through a page
//! fault, possibly a hard one from the pagefile.
//!
//! That makes periodic trimming a net loss: it buys a cosmetic number and pays
//! for it in faults, forever. The one defensible use is a *single* call once
//! start-up is over, on a process that then sits idle for a long time: CRT
//! initialisation, DLL relocation and resource loading touch pages that are
//! never read again, and there is no reason to keep them resident for hours.
//!
//! Windows already does this on its own under memory pressure, so treat this
//! as an optimisation for how the app *looks* while idle, not for how much
//! memory it actually costs the system.

use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::core::Result;

/// Releases the pages touched during start-up from the working set.
///
/// Prefer this over `SetProcessWorkingSetSize` with explicit limits: passing
/// a minimum like 100 KB sets a *quota* rather than performing a one-off trim,
/// and a process kept under an artificially low quota faults continuously.
/// `EmptyWorkingSet` is the documented way to say "trim now, once".
pub fn trim_working_set() -> Result<()> {
    unsafe { EmptyWorkingSet(GetCurrentProcess()) }
}
