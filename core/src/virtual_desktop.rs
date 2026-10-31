//! Virtual desktops: switch, create, move a window to one, pin a window to
//! all of them — backed by the undocumented `IVirtualDesktopManagerInternal`
//! COM interface every third-party virtual-desktop tool (including the
//! GUIPro sibling this workspace ports, HulK) ends up reverse-engineering,
//! because Microsoft has never shipped a public API for any of this.
//!
//! # Two vtable shapes, not one
//!
//! The undocumented interface's vtable has changed shape across Windows
//! 10/11 builds — confirmed twice, once inserting
//! `SwitchDesktopAndMoveForegroundView` right after `SwitchDesktop` and once
//! inserting `MoveDesktop` right after `CreateDesktopW`. A caller that
//! assumes one fixed method order and just tries several interface IDs
//! against it — which is what HulK's own `virtualdesktop.cpp` does — gets
//! away with it only as long as it never calls anything past `SwitchDesktop`
//! on an older build: every call from `CreateDesktopW` onward lands one or
//! two vtable slots off from what it meant to call, silently invoking the
//! wrong method with the wrong arguments.
//!
//! This module declares both shapes as distinct Rust types —
//! [`legacy::IVirtualDesktopManagerInternal`] (the older, 10-method layout)
//! and [`modern::IVirtualDesktopManagerInternal`] (the current, 24H2/25H2
//! layout) — and [`acquire`] probes a table of known interface IDs, newest
//! first, pairing each one with the shape it is known (or, for the oldest
//! ones, believed) to carry. [`Manager`] then dispatches every operation to
//! whichever shape was actually found, so a call is never made through a
//! struct whose method order does not match the object answering it.
//!
//! # Threading
//!
//! Every handle here is a raw COM pointer, not `Send`/`Sync`. State lives in
//! a `thread_local!`, the same rule this workspace already follows for
//! low-level-hook state (see `y_core::hotkey`'s own doc comment) — not
//! because this runs inside a hook, but because nothing here is ever called
//! from more than one thread (y-snap's own message-loop thread) and a
//! `thread_local!` says so directly instead of reaching for a `Mutex` no
//! caller could ever contend.
//!
//! # Never released
//!
//! `CLSID_ImmersiveShell` is activated with `CLSCTX_LOCAL_SERVER`: every
//! interface this module ever holds — the shell service provider itself,
//! the manager, the view collection, the pinned-apps service — is,
//! transitively, a proxy to an object living in `explorer.exe`, not in this
//! process. Releasing a proxy is not free: it is an RPC call back into
//! Explorer, made synchronously, and Explorer does not always answer
//! instantly. Doing that at the moment y-snap is quitting — right as its
//! message loop is winding down — was exactly what made quitting look like
//! the whole desktop had frozen for a few seconds: the process was sitting
//! in a blocking `Release()`/`CoUninitialize()` round trip, not actually
//! stuck, but nothing else was visibly moving either.
//!
//! The fix is to never make that round trip in the first place. The
//! thread-local [`Manager`] this module caches, and the short-lived
//! `IServiceProvider`/rejected-candidate values [`acquire`] touches along
//! the way, are wrapped in [`ManuallyDrop`] specifically so their `Release`
//! never runs — not at shutdown, not ever. This does leak one COM reference
//! per acquired interface for the life of the process, which is the
//! deliberate trade: a process that is going to exit anyway does not need a
//! graceful COM teardown, and Windows reclaims everything (including the
//! server-side proxy state) the moment the process itself dies, without a
//! synchronous round trip through this module at all. Per-call objects
//! returned from inside [`ShapeOps`] (a desktop, a view, an object array)
//! are *not* covered by this and still drop normally — those are acquired
//! and released continuously over a session, not once at start-up, so
//! leaking them would be an actual unbounded leak instead of a bounded,
//! once-per-process one.
//!
//! # What this is adapted from
//!
//! `../guipro-git/HulK/src/virtualdesktop.cpp` (the CoCreateInstance/
//! QueryService dance, the candidate-probing idea) and
//! `../guipro-git/HulK/plugins/desktops/src/win10.h` (the older interface
//! shape). Not a line-for-line port: HulK validates a candidate with one
//! live `GetCount()` call and otherwise trusts the shape it guessed; this
//! module instead pins the *correct* shape to each known interface ID up
//! front, which is what actually avoids the vtable-slot-drift bug HulK's own
//! comments describe hitting.

// COM method names keep the original PascalCase throughout this module —
// grep-able against the C++ sources they are adapted from — the same
// convention the `windows` crate's own generated bindings use.
#![allow(non_snake_case)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::mem::ManuallyDrop;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, CoCreateInstance, CoInitialize, IServiceProvider,
};
use windows::Win32::UI::Shell::Common::IObjectArray;
use windows::core::{GUID, Interface, Result};

/// Which way [`switch`] moves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Left,
    Right,
}

// GUIDs written the same way the GUIPro C++ sources declare them —
// `data1, data2, data3, data4[0], data4[1], ...` — so each one can be
// diffed by eye against `virtualdesktop.cpp`/`win10.h` rather than against
// an opaque packed `u128`.
const CLSID_IMMERSIVE_SHELL: GUID = GUID::from_values(
    0xC2F03A33,
    0x21F5,
    0x47FA,
    [0xB4, 0xBB, 0x15, 0x63, 0x62, 0xA2, 0xF2, 0x39],
);
const CLSID_VIRTUAL_DESKTOP_API_UNKNOWN: GUID = GUID::from_values(
    0xC5E0CDCA,
    0x7B6E,
    0x41B2,
    [0x9F, 0xC4, 0xD9, 0x39, 0x75, 0xCC, 0x46, 0x7B],
);
const CLSID_VIRTUAL_DESKTOP_PINNED_APPS: GUID = GUID::from_values(
    0xB5A399E7,
    0x1C87,
    0x46B8,
    [0x88, 0xE9, 0xFC, 0x57, 0x47, 0xB1, 0x71, 0xBD],
);
const IID_APPLICATION_VIEW_COLLECTION: GUID = GUID::from_values(
    0x1841C6D7,
    0x4F9D,
    0x42C0,
    [0xAF, 0x41, 0x87, 0x47, 0x53, 0x8F, 0x10, 0xE5],
);
const IID_VIRTUAL_DESKTOP_PINNED_APPS: GUID = GUID::from_values(
    0x4CE81583,
    0x1E4C,
    0x4632,
    [0xA6, 0x21, 0x07, 0xA5, 0x35, 0x43, 0x14, 0x8F],
);

/// Known `IVirtualDesktopManagerInternal` interface IDs, newest first, each
/// paired with the vtable shape it carries. `GetCount()` is the only method
/// [`probe`] calls before trusting one; every method past that only ever
/// runs through the shape recorded here, not guessed at.
///
/// The `_15063` entry is best-effort: per HulK's own comment, this interface
/// ID is confirmed to answer across builds from 15063 (Fall Creators Update)
/// through roughly the low 22000s, but the *exact* vtable shape across that
/// whole range is not independently confirmed — it is assumed to still be
/// the legacy shape here. If virtual desktop actions crash on a build in
/// that window, this is the first thing to re-check.
#[allow(non_upper_case_globals)]
mod candidates {
    use super::GUID;
    use super::Shape;

    pub struct Candidate {
        pub label: &'static str,
        pub iid: GUID,
        pub shape: Shape,
    }

    pub const CANDIDATES: &[Candidate] = &[
        Candidate {
            label: "26100 (24H2/25H2)",
            iid: GUID::from_values(
                0x53F5CA0B,
                0x158F,
                0x4124,
                [0x90, 0x0C, 0x05, 0x71, 0x58, 0x06, 0x0B, 0x27],
            ),
            shape: Shape::Modern,
        },
        Candidate {
            label: "15063 (Fall Creators Update+, best-effort)",
            iid: GUID::from_values(
                0xB2F925B9,
                0x5A0F,
                0x4D2E,
                [0x9F, 0x4D, 0x2B, 0x15, 0x07, 0x59, 0x3C, 0x10],
            ),
            shape: Shape::Legacy,
        },
        Candidate {
            label: "14393 (Anniversary Update)",
            iid: GUID::from_values(
                0xf31574d6,
                0xb682,
                0x4cdc,
                [0xbd, 0x56, 0x18, 0x27, 0x86, 0x0a, 0xbe, 0xc6],
            ),
            shape: Shape::Legacy,
        },
        Candidate {
            label: "10240 (November Update)",
            iid: GUID::from_values(
                0xAF8DA486,
                0x95BB,
                0x4460,
                [0xB3, 0xB7, 0x6E, 0x7A, 0x6B, 0x29, 0x62, 0xB5],
            ),
            shape: Shape::Legacy,
        },
        Candidate {
            label: "10130 (early Win10 preview)",
            iid: GUID::from_values(
                0xEF9F1A6C,
                0xD3CC,
                0x4358,
                [0xB7, 0x12, 0xF8, 0x4B, 0x63, 0x5B, 0xEB, 0xE7],
            ),
            shape: Shape::Legacy,
        },
    ];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    Legacy,
    Modern,
}

/// The older `IVirtualDesktopManagerInternal`/`IVirtualDesktop` vtable
/// shape: no `SwitchDesktopAndMoveForegroundView`, no `MoveDesktop`. Matches
/// `plugins/desktops/src/win10.h` in the GUIPro sources.
mod legacy {
    use super::c_void;
    use windows_core::*;

    #[interface("FF72FFDD-BE7E-43FC-9C03-AD81681E88E4")]
    pub unsafe trait IVirtualDesktop: IUnknown {
        pub fn IsViewVisible(&self, view: *mut c_void, visible: *mut i32) -> HRESULT;
        pub fn GetID(&self, id: *mut GUID) -> HRESULT;
    }

    /// Interface ID kept arbitrary/unused: never obtained through
    /// `QueryInterface`, only ever constructed from a raw pointer already
    /// known to be an [`IApplicationView`] — see this module's parent doc
    /// comment on why `GetViewForHwnd` needs no `riid` at all.
    #[interface("372E1D3B-38D3-42E4-A15B-8AB2B178F513")]
    pub unsafe trait IApplicationView: IUnknown {}

    #[interface("AF8DA486-95BB-4460-B3B7-6E7A6B2962B5")]
    pub unsafe trait IVirtualDesktopManagerInternal: IUnknown {
        pub fn GetCount(&self, count: *mut u32) -> HRESULT;
        pub fn MoveViewToDesktop(&self, view: *mut c_void, desktop: *mut c_void) -> HRESULT;
        pub fn CanViewMoveDesktops(&self, view: *mut c_void, can_move: *mut i32) -> HRESULT;
        pub fn GetCurrentDesktop(&self, desktop: *mut *mut c_void) -> HRESULT;
        pub fn GetDesktops(&self, desktops: *mut *mut c_void) -> HRESULT;
        pub fn GetAdjacentDesktop(
            &self,
            reference: *mut c_void,
            direction: u32,
            adjacent: *mut *mut c_void,
        ) -> HRESULT;
        pub fn SwitchDesktop(&self, desktop: *mut c_void) -> HRESULT;
        pub fn CreateDesktopW(&self, desktop: *mut *mut c_void) -> HRESULT;
        pub fn RemoveDesktop(&self, remove: *mut c_void, fallback: *mut c_void) -> HRESULT;
        pub fn FindDesktop(&self, id: *const GUID, desktop: *mut *mut c_void) -> HRESULT;
    }

    #[interface("1841C6D7-4F9D-42C0-AF41-8747538F10E5")]
    pub unsafe trait IApplicationViewCollection: IUnknown {
        pub fn GetViews(&self, out_views: *mut *mut c_void) -> HRESULT;
        pub fn GetViewsByZOrder(&self, out_views: *mut *mut c_void) -> HRESULT;
        pub fn GetViewsByAppUserModelId(
            &self,
            id: windows::core::PCWSTR,
            out_views: *mut *mut c_void,
        ) -> HRESULT;
        pub fn GetViewForHwnd(
            &self,
            window: windows::Win32::Foundation::HWND,
            out_view: *mut *mut c_void,
        ) -> HRESULT;
    }

    #[interface("4CE81583-1E4C-4632-A621-07A53543148F")]
    pub unsafe trait IVirtualDesktopPinnedApps: IUnknown {
        pub fn IsAppPinned(&self, app_id: windows::core::PCWSTR, is_pinned: *mut i32) -> HRESULT;
        pub fn PinApp(&self, app_id: windows::core::PCWSTR) -> HRESULT;
        pub fn UnpinApp(&self, app_id: windows::core::PCWSTR) -> HRESULT;
        pub fn IsViewPinned(&self, view: *mut c_void, is_pinned: *mut i32) -> HRESULT;
        pub fn PinView(&self, view: *mut c_void) -> HRESULT;
        pub fn UnpinView(&self, view: *mut c_void) -> HRESULT;
    }
}

/// The current (24H2/25H2-confirmed) vtable shape:
/// `SwitchDesktopAndMoveForegroundView` inserted after `SwitchDesktop`,
/// `MoveDesktop` inserted after `CreateDesktopW`. Matches
/// `HulK/src/virtualdesktop.cpp`.
mod modern {
    use super::c_void;
    use windows_core::*;

    #[interface("3F07F4BE-B107-441A-AF0F-39D82529072C")]
    pub unsafe trait IVirtualDesktop: IUnknown {
        pub fn IsViewVisible(&self, view: *mut c_void, visible: *mut u32) -> HRESULT;
        pub fn GetID(&self, id: *mut GUID) -> HRESULT;
    }

    #[interface("53F5CA0B-158F-4124-900C-057158060B27")]
    pub unsafe trait IVirtualDesktopManagerInternal: IUnknown {
        pub fn GetCount(&self, count: *mut u32) -> HRESULT;
        pub fn MoveViewToDesktop(&self, view: *mut c_void, desktop: *mut c_void) -> HRESULT;
        pub fn CanViewMoveDesktops(&self, view: *mut c_void, can_move: *mut i32) -> HRESULT;
        pub fn GetCurrentDesktop(&self, desktop: *mut *mut c_void) -> HRESULT;
        pub fn GetDesktops(&self, desktops: *mut *mut c_void) -> HRESULT;
        pub fn GetAdjacentDesktop(
            &self,
            reference: *mut c_void,
            direction: u32,
            adjacent: *mut *mut c_void,
        ) -> HRESULT;
        pub fn SwitchDesktop(&self, desktop: *mut c_void) -> HRESULT;
        pub fn SwitchDesktopAndMoveForegroundView(&self, desktop: *mut c_void) -> HRESULT;
        pub fn CreateDesktopW(&self, desktop: *mut *mut c_void) -> HRESULT;
        pub fn MoveDesktop(&self, desktop: *mut c_void, index: u32) -> HRESULT;
        pub fn RemoveDesktop(&self, remove: *mut c_void, fallback: *mut c_void) -> HRESULT;
        pub fn FindDesktop(&self, id: *const GUID, desktop: *mut *mut c_void) -> HRESULT;
    }
}

/// One resolved manager: the correctly-shaped `IVirtualDesktopManagerInternal`
/// pointer plus the auxiliary services HulK's own `GetVirtualDesktopManager`
/// acquires alongside it. `IApplicationViewCollection`/`IVirtualDesktopPinnedApps`
/// use the legacy-shape declarations regardless of which manager shape was
/// picked — both are unaffected by the manager-internal reshuffle and their
/// declared shapes have not changed across the same builds.
enum ManagerHandle {
    Legacy(legacy::IVirtualDesktopManagerInternal),
    Modern(modern::IVirtualDesktopManagerInternal),
}

struct Manager {
    handle: ManagerHandle,
    view_collection: Option<legacy::IApplicationViewCollection>,
    pinned_apps: Option<legacy::IVirtualDesktopPinnedApps>,
}

// `ManuallyDrop`, not a plain `Manager` — see this module's own "Never
// released" doc section for why `Manager`'s `Release()` calls must never
// run, including at thread/process teardown.
thread_local! {
    static MANAGER: RefCell<Option<ManuallyDrop<Manager>>> = const { RefCell::new(None) };
}

/// Live, SEH-free validation of a freshly-acquired candidate: `GetCount()`
/// sits at the same vtable slot in every shape this module knows about, so
/// a candidate that answers here at least has a genuine
/// `IVirtualDesktopManagerInternal`-family object behind it — it does not,
/// on its own, prove the rest of the shape assigned to it in
/// [`candidates::CANDIDATES`] is correct, only that slot 0 is.
fn probe(handle: &ManagerHandle) -> bool {
    let mut count = 0u32;
    let hr = match handle {
        ManagerHandle::Legacy(manager) => unsafe { manager.GetCount(&mut count) },
        ManagerHandle::Modern(manager) => unsafe { manager.GetCount(&mut count) },
    };
    hr.is_ok()
}

/// Acquires (or reuses) the thread-local [`Manager`], running the same
/// `CoInitialize` → `CLSID_ImmersiveShell` → candidate-probing sequence as
/// HulK's `GetVirtualDesktopManager`, once per thread.
fn with_manager<T>(f: impl FnOnce(&Manager) -> Result<T>) -> Result<T> {
    MANAGER.with(|cell| {
        {
            let existing = cell.borrow();
            if let Some(manager) = existing.as_ref() {
                return f(manager);
            }
        }

        let manager = acquire()?;
        let result = f(&manager);
        *cell.borrow_mut() = Some(ManuallyDrop::new(manager));
        result
    })
}

fn acquire() -> Result<Manager> {
    // Matches HulK's own comment: CoInitialize, not CoInitializeEx — this
    // thread must not already be COINIT_MULTITHREADED. Never matched by a
    // `CoUninitialize()` — see this module's "Never released" doc section.
    unsafe { CoInitialize(None).ok()? };

    // `ManuallyDrop`: this is itself a proxy to an out-of-process object
    // (`CLSCTX_LOCAL_SERVER`) — see "Never released". Every exit path from
    // this function, success or failure, must not release it.
    let shell = ManuallyDrop::new(unsafe {
        CoCreateInstance::<_, IServiceProvider>(&CLSID_IMMERSIVE_SHELL, None, CLSCTX_LOCAL_SERVER)?
    });

    let mut handle = None;
    for candidate in candidates::CANDIDATES {
        let Ok(raw) =
            (unsafe { query_service(&shell, &CLSID_VIRTUAL_DESKTOP_API_UNKNOWN, &candidate.iid) })
        else {
            continue;
        };

        // `ManuallyDrop` here too: a rejected candidate must not release
        // either, for the same reason — probing runs once per process, so
        // leaking a rejected one costs nothing that matters.
        let candidate_handle = ManuallyDrop::new(match candidate.shape {
            Shape::Legacy => ManagerHandle::Legacy(unsafe {
                legacy::IVirtualDesktopManagerInternal::from_raw(raw)
            }),
            Shape::Modern => ManagerHandle::Modern(unsafe {
                modern::IVirtualDesktopManagerInternal::from_raw(raw)
            }),
        });

        if probe(&candidate_handle) {
            let _ = candidate.label; // kept for future diagnostics/logging
            handle = Some(ManuallyDrop::into_inner(candidate_handle));
            break;
        }
    }

    let Some(handle) = handle else {
        return Err(windows::core::Error::from(
            windows::Win32::Foundation::E_NOINTERFACE,
        ));
    };

    // Non-fatal if either of these fails — only the functions that need them
    // (move-to-desktop, pin/unpin) are affected; see `with_manager`'s own
    // callers for how they degrade.
    let view_collection = unsafe {
        query_service(
            &shell,
            &IID_APPLICATION_VIEW_COLLECTION,
            &IID_APPLICATION_VIEW_COLLECTION,
        )
    }
    .ok()
    .map(|raw| unsafe { legacy::IApplicationViewCollection::from_raw(raw) });

    let pinned_apps = unsafe {
        query_service(
            &shell,
            &CLSID_VIRTUAL_DESKTOP_PINNED_APPS,
            &IID_VIRTUAL_DESKTOP_PINNED_APPS,
        )
    }
    .ok()
    .map(|raw| unsafe { legacy::IVirtualDesktopPinnedApps::from_raw(raw) });

    Ok(Manager {
        handle,
        view_collection,
        pinned_apps,
    })
}

/// Calls `IServiceProvider::QueryService` with an explicit, runtime `riid` —
/// not the convenience `QueryService<T>()` wrapper the `windows` crate
/// generates, which always requests `T::IID`. Every legacy-era candidate in
/// [`candidates::CANDIDATES`] shares one Rust type
/// ([`legacy::IVirtualDesktopManagerInternal`]) but has a *different* real
/// interface ID, so the ID actually requested has to come from the
/// candidate table, not from the Rust type receiving the result.
unsafe fn query_service(
    provider: &IServiceProvider,
    service: &GUID,
    iid: &GUID,
) -> Result<*mut c_void> {
    let mut result: *mut c_void = core::ptr::null_mut();
    unsafe {
        (Interface::vtable(provider).QueryService)(
            Interface::as_raw(provider),
            service,
            iid,
            &mut result,
        )
        .ok()?;
    }
    Ok(result)
}

// AdjacentDesktop's two values — kept as plain `u32` on the wire (the
// C++ original's `enum AdjacentDesktop`) rather than a typed Rust enum,
// since it only ever crosses the FFI boundary as one of these two numbers.
const LEFT_DIRECTION: u32 = 3;
const RIGHT_DIRECTION: u32 = 4;

/// The operations [`switch`], [`create_desktop`] and friends need, phrased
/// once and implemented per vtable shape — see this module's own doc
/// comment for why there have to be two implementations rather than one.
/// `Self::Desktop` is that shape's own `IVirtualDesktop`-equivalent type:
/// [`legacy::IVirtualDesktop`] or [`modern::IVirtualDesktop`], never mixed
/// with the other shape's manager.
trait ShapeOps {
    type Desktop: Interface;

    fn desktop_count(&self) -> Result<u32>;
    fn current_desktop(&self) -> Result<Self::Desktop>;
    fn desktops(&self) -> Result<IObjectArray>;
    fn adjacent_desktop(&self, reference: &Self::Desktop, direction: u32) -> Result<Self::Desktop>;
    fn switch_desktop(&self, desktop: &Self::Desktop) -> Result<()>;
    fn create_desktop(&self) -> Result<Self::Desktop>;
    fn move_view_to_desktop(
        &self,
        view: &legacy::IApplicationView,
        desktop: &Self::Desktop,
    ) -> Result<()>;
    fn desktop_id(&self, desktop: &Self::Desktop) -> Result<GUID>;
}

impl ShapeOps for legacy::IVirtualDesktopManagerInternal {
    type Desktop = legacy::IVirtualDesktop;

    fn desktop_count(&self) -> Result<u32> {
        let mut count = 0u32;
        unsafe { self.GetCount(&mut count).ok()? };
        Ok(count)
    }

    fn current_desktop(&self) -> Result<Self::Desktop> {
        let mut raw = core::ptr::null_mut();
        unsafe { self.GetCurrentDesktop(&mut raw).ok()? };
        Ok(unsafe { Self::Desktop::from_raw(raw) })
    }

    fn desktops(&self) -> Result<IObjectArray> {
        let mut raw = core::ptr::null_mut();
        unsafe { self.GetDesktops(&mut raw).ok()? };
        Ok(unsafe { IObjectArray::from_raw(raw) })
    }

    fn adjacent_desktop(&self, reference: &Self::Desktop, direction: u32) -> Result<Self::Desktop> {
        let mut raw = core::ptr::null_mut();
        unsafe {
            self.GetAdjacentDesktop(Interface::as_raw(reference), direction, &mut raw)
                .ok()?;
        }
        Ok(unsafe { Self::Desktop::from_raw(raw) })
    }

    fn switch_desktop(&self, desktop: &Self::Desktop) -> Result<()> {
        unsafe { self.SwitchDesktop(Interface::as_raw(desktop)).ok() }
    }

    fn create_desktop(&self) -> Result<Self::Desktop> {
        let mut raw = core::ptr::null_mut();
        unsafe { self.CreateDesktopW(&mut raw).ok()? };
        Ok(unsafe { Self::Desktop::from_raw(raw) })
    }

    fn move_view_to_desktop(
        &self,
        view: &legacy::IApplicationView,
        desktop: &Self::Desktop,
    ) -> Result<()> {
        unsafe {
            self.MoveViewToDesktop(Interface::as_raw(view), Interface::as_raw(desktop))
                .ok()
        }
    }

    fn desktop_id(&self, desktop: &Self::Desktop) -> Result<GUID> {
        let mut id = GUID::zeroed();
        unsafe { desktop.GetID(&mut id).ok()? };
        Ok(id)
    }
}

impl ShapeOps for modern::IVirtualDesktopManagerInternal {
    type Desktop = modern::IVirtualDesktop;

    fn desktop_count(&self) -> Result<u32> {
        let mut count = 0u32;
        unsafe { self.GetCount(&mut count).ok()? };
        Ok(count)
    }

    fn current_desktop(&self) -> Result<Self::Desktop> {
        let mut raw = core::ptr::null_mut();
        unsafe { self.GetCurrentDesktop(&mut raw).ok()? };
        Ok(unsafe { Self::Desktop::from_raw(raw) })
    }

    fn desktops(&self) -> Result<IObjectArray> {
        let mut raw = core::ptr::null_mut();
        unsafe { self.GetDesktops(&mut raw).ok()? };
        Ok(unsafe { IObjectArray::from_raw(raw) })
    }

    fn adjacent_desktop(&self, reference: &Self::Desktop, direction: u32) -> Result<Self::Desktop> {
        let mut raw = core::ptr::null_mut();
        unsafe {
            self.GetAdjacentDesktop(Interface::as_raw(reference), direction, &mut raw)
                .ok()?;
        }
        Ok(unsafe { Self::Desktop::from_raw(raw) })
    }

    fn switch_desktop(&self, desktop: &Self::Desktop) -> Result<()> {
        unsafe { self.SwitchDesktop(Interface::as_raw(desktop)).ok() }
    }

    fn create_desktop(&self) -> Result<Self::Desktop> {
        let mut raw = core::ptr::null_mut();
        unsafe { self.CreateDesktopW(&mut raw).ok()? };
        Ok(unsafe { Self::Desktop::from_raw(raw) })
    }

    fn move_view_to_desktop(
        &self,
        view: &legacy::IApplicationView,
        desktop: &Self::Desktop,
    ) -> Result<()> {
        unsafe {
            self.MoveViewToDesktop(Interface::as_raw(view), Interface::as_raw(desktop))
                .ok()
        }
    }

    fn desktop_id(&self, desktop: &Self::Desktop) -> Result<GUID> {
        let mut id = GUID::zeroed();
        unsafe { desktop.GetID(&mut id).ok()? };
        Ok(id)
    }
}

/// 0-based index of `target`'s ID within `manager`'s own `GetDesktops()`
/// list, by comparing `GetID()` one at a time — the only way to turn a
/// desktop reference into a position, since nothing here exposes one
/// directly. Shared by [`current_desktop_index`] and [`switch`]'s own
/// wraparound handling.
fn index_of<M: ShapeOps>(manager: &M, target: &GUID) -> Result<Option<u32>> {
    let desktops = manager.desktops()?;
    let count = unsafe { desktops.GetCount()? };
    for i in 0..count {
        let Ok(desktop) = (unsafe { desktops.GetAt::<M::Desktop>(i) }) else {
            continue;
        };
        if manager.desktop_id(&desktop)? == *target {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

/// The desktop adjacent to the current one in `dir`, wrapping around to the
/// other end of the list when there is none — mirrors HulK's own
/// `getAdjacentVirtualDesktop`.
fn adjacent<M: ShapeOps>(manager: &M, dir: Direction) -> Result<M::Desktop> {
    let current = manager.current_desktop()?;
    let direction = match dir {
        Direction::Left => LEFT_DIRECTION,
        Direction::Right => RIGHT_DIRECTION,
    };

    if let Ok(next) = manager.adjacent_desktop(&current, direction) {
        return Ok(next);
    }

    let desktops = manager.desktops()?;
    let count = unsafe { desktops.GetCount()? };
    if count == 0 {
        return Err(windows::core::Error::from(
            windows::Win32::Foundation::E_FAIL,
        ));
    }
    let index = match dir {
        Direction::Left => count - 1,
        Direction::Right => 0,
    };
    Ok(unsafe { desktops.GetAt::<M::Desktop>(index)? })
}

/// Parks focus on the desktop (`Progman`) before switching, and minimizes it
/// again right after — see HulK's own `switchVirtualDesktop` comment: this
/// stops the taskbar's foreground-window icon from "blinking" mid-switch,
/// while `SwitchDesktop` resolves which window becomes foreground on the new
/// desktop. `AttachThreadInput` is what makes a background process's
/// `SetForegroundWindow` call actually take effect, bypassing Windows'
/// usual foreground-lock rule.
fn with_parked_focus(f: impl FnOnce()) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, SW_MINIMIZE,
        SetForegroundWindow, ShowWindow,
    };

    let progman = unsafe {
        FindWindowW(
            windows::core::w!("Progman"),
            windows::core::w!("Program Manager"),
        )
    }
    .unwrap_or(HWND::default());
    if progman.is_invalid() {
        f();
        return;
    }

    let current_thread = unsafe { GetCurrentThreadId() };
    let foreground = unsafe { GetForegroundWindow() };
    let foreground_thread = unsafe { GetWindowThreadProcessId(foreground, None) };
    let desktop_thread = unsafe { GetWindowThreadProcessId(progman, None) };

    unsafe {
        let _ = AttachThreadInput(desktop_thread, current_thread, true);
        let _ = AttachThreadInput(foreground_thread, current_thread, true);
        let _ = SetForegroundWindow(progman);
    }

    f();

    unsafe {
        let _ = AttachThreadInput(desktop_thread, current_thread, false);
        let _ = AttachThreadInput(foreground_thread, current_thread, false);
        let _ = ShowWindow(progman, SW_MINIMIZE);
    }
}

fn switch_desktop_generic<M: ShapeOps>(
    manager: &M,
    view_collection: Option<&legacy::IApplicationViewCollection>,
    dir: Direction,
    move_window: Option<HWND>,
) -> Result<()> {
    let next = adjacent(manager, dir)?;
    switch_to_desktop(manager, view_collection, &next, move_window)
}

/// Switches to `next`, carrying `move_window` along first.
fn switch_to_desktop<M: ShapeOps>(
    manager: &M,
    view_collection: Option<&legacy::IApplicationViewCollection>,
    next: &M::Desktop,
    move_window: Option<HWND>,
) -> Result<()> {
    // Move first, same as HulK: the destination desktop should already have
    // the window on it by the time `SwitchDesktop` makes it visible.
    if let (Some(hwnd), Some(collection)) = (move_window, view_collection) {
        let mut raw = core::ptr::null_mut();
        if unsafe { collection.GetViewForHwnd(hwnd, &mut raw) }.is_ok() && !raw.is_null() {
            let view = unsafe { legacy::IApplicationView::from_raw(raw) };
            let _ = manager.move_view_to_desktop(&view, next);
        }
    }

    with_parked_focus(|| {
        let _ = manager.switch_desktop(next);
    });

    Ok(())
}

fn move_window_to_desktop_generic<M: ShapeOps>(
    manager: &M,
    view_collection: &legacy::IApplicationViewCollection,
    hwnd: HWND,
    index: u32,
) -> Result<()> {
    let mut raw = core::ptr::null_mut();
    unsafe { view_collection.GetViewForHwnd(hwnd, &mut raw).ok()? };
    if raw.is_null() {
        return Err(windows::core::Error::from(
            windows::Win32::Foundation::E_FAIL,
        ));
    }
    let view = unsafe { legacy::IApplicationView::from_raw(raw) };

    let desktops = manager.desktops()?;
    let desktop = unsafe { desktops.GetAt::<M::Desktop>(index)? };
    manager.move_view_to_desktop(&view, &desktop)
}

impl Manager {
    fn desktop_count(&self) -> Result<u32> {
        match &self.handle {
            ManagerHandle::Legacy(m) => m.desktop_count(),
            ManagerHandle::Modern(m) => m.desktop_count(),
        }
    }

    fn current_desktop_index(&self) -> Result<Option<u32>> {
        match &self.handle {
            ManagerHandle::Legacy(m) => {
                let id = m.desktop_id(&m.current_desktop()?)?;
                index_of(m, &id)
            }
            ManagerHandle::Modern(m) => {
                let id = m.desktop_id(&m.current_desktop()?)?;
                index_of(m, &id)
            }
        }
    }

    fn switch(&self, dir: Direction, move_window: Option<HWND>) -> Result<()> {
        match &self.handle {
            ManagerHandle::Legacy(m) => {
                switch_desktop_generic(m, self.view_collection.as_ref(), dir, move_window)
            }
            ManagerHandle::Modern(m) => {
                switch_desktop_generic(m, self.view_collection.as_ref(), dir, move_window)
            }
        }
    }

    fn switch_to_index(&self, index: u32, move_window: Option<HWND>) -> Result<()> {
        match &self.handle {
            ManagerHandle::Legacy(m) => {
                let desktop = unsafe { m.desktops()?.GetAt(index)? };
                switch_to_desktop(m, self.view_collection.as_ref(), &desktop, move_window)
            }
            ManagerHandle::Modern(m) => {
                let desktop = unsafe { m.desktops()?.GetAt(index)? };
                switch_to_desktop(m, self.view_collection.as_ref(), &desktop, move_window)
            }
        }
    }

    fn create_desktop(&self) -> Result<()> {
        match &self.handle {
            ManagerHandle::Legacy(m) => m.create_desktop().map(drop),
            ManagerHandle::Modern(m) => m.create_desktop().map(drop),
        }
    }

    fn move_window_to_desktop(&self, hwnd: HWND, index: u32) -> Result<()> {
        let Some(collection) = self.view_collection.as_ref() else {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_NOINTERFACE,
            ));
        };
        match &self.handle {
            ManagerHandle::Legacy(m) => move_window_to_desktop_generic(m, collection, hwnd, index),
            ManagerHandle::Modern(m) => move_window_to_desktop_generic(m, collection, hwnd, index),
        }
    }

    fn application_view(&self, hwnd: HWND) -> Result<legacy::IApplicationView> {
        let Some(collection) = self.view_collection.as_ref() else {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_NOINTERFACE,
            ));
        };
        let mut raw = core::ptr::null_mut();
        unsafe { collection.GetViewForHwnd(hwnd, &mut raw).ok()? };
        if raw.is_null() {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_FAIL,
            ));
        }
        Ok(unsafe { legacy::IApplicationView::from_raw(raw) })
    }

    fn is_window_pinned(&self, hwnd: HWND) -> Result<bool> {
        let Some(pinned_apps) = self.pinned_apps.as_ref() else {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_NOINTERFACE,
            ));
        };
        let view = self.application_view(hwnd)?;
        let mut is_pinned = 0i32;
        unsafe {
            pinned_apps
                .IsViewPinned(Interface::as_raw(&view), &mut is_pinned)
                .ok()?;
        }
        Ok(is_pinned != 0)
    }

    fn pin_window(&self, hwnd: HWND) -> Result<()> {
        let Some(pinned_apps) = self.pinned_apps.as_ref() else {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_NOINTERFACE,
            ));
        };
        let view = self.application_view(hwnd)?;
        unsafe { pinned_apps.PinView(Interface::as_raw(&view)).ok() }
    }

    fn unpin_window(&self, hwnd: HWND) -> Result<()> {
        let Some(pinned_apps) = self.pinned_apps.as_ref() else {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_NOINTERFACE,
            ));
        };
        let view = self.application_view(hwnd)?;
        unsafe { pinned_apps.UnpinView(Interface::as_raw(&view)).ok() }
    }
}

/// How many virtual desktops exist, or `None` if the interface could not be
/// acquired at all (an unrecognised Windows build — see this module's own
/// doc comment).
pub fn desktop_count() -> Option<u32> {
    with_manager(|manager| manager.desktop_count()).ok()
}

/// 0-based index of the current desktop within [`desktop_count`]'s own
/// list, or `None` if it could not be determined.
pub fn current_desktop_index() -> Option<u32> {
    with_manager(|manager| manager.current_desktop_index())
        .ok()
        .flatten()
}

/// Switches to the desktop adjacent to the current one, wrapping around at
/// either end. `move_window`, if given, is carried along to the destination
/// desktop first — see [`with_parked_focus`]'s doc comment for the
/// foreground-window dance this does around the actual switch.
pub fn switch(dir: Direction, move_window: Option<HWND>) -> Result<()> {
    with_manager(|manager| manager.switch(dir, move_window))
}

/// Switches to the desktop at `index` (0-based), carrying `move_window` along
/// first, as [`switch`] does.
pub fn switch_to(index: u32, move_window: Option<HWND>) -> Result<()> {
    with_manager(|manager| manager.switch_to_index(index, move_window))
}

/// Creates a new virtual desktop. Does not switch to it.
pub fn create_desktop() -> Result<()> {
    with_manager(|manager| manager.create_desktop())
}

/// Moves `hwnd` to the desktop at `index` (0-based) without switching to it.
pub fn move_window_to_desktop(hwnd: HWND, index: u32) -> Result<()> {
    with_manager(|manager| manager.move_window_to_desktop(hwnd, index))
}

/// Whether `hwnd` is pinned to every desktop.
pub fn is_window_pinned(hwnd: HWND) -> Result<bool> {
    with_manager(|manager| manager.is_window_pinned(hwnd))
}

/// Pins `hwnd` so it shows on every virtual desktop.
pub fn pin_window(hwnd: HWND) -> Result<()> {
    with_manager(|manager| manager.pin_window(hwnd))
}

/// Unpins `hwnd`, back to showing only on the desktop it is actually on.
pub fn unpin_window(hwnd: HWND) -> Result<()> {
    with_manager(|manager| manager.unpin_window(hwnd))
}

/// Pins `hwnd` if it is not pinned, unpins it if it is.
pub fn toggle_window_pinned(hwnd: HWND) -> Result<()> {
    with_manager(|manager| {
        if manager.is_window_pinned(hwnd)? {
            manager.unpin_window(hwnd)
        } else {
            manager.pin_window(hwnd)
        }
    })
}
