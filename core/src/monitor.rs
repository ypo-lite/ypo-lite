//! Monitor geometry: where the screens are, how much of them is usable, and
//! at what scale.
//!
//! Anything that puts pixels on screen without a window manager to help —
//! an overlay, a snapping grid, a popup near the cursor — needs this. All
//! coordinates are physical pixels: the manifest declares per-monitor v2 DPI
//! awareness, so Windows does not scale anything on our behalf.

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint, MonitorFromWindow,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetForegroundWindow, IsWindowVisible,
};
use windows::core::Result;

/// The reference DPI: 100% scaling.
pub const DEFAULT_DPI: u32 = 96;

/// A monitor's geometry, sampled at the time it was queried. Monitors come
/// and go (docking, remote sessions, a projector); re-query rather than cache.
#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    /// The whole screen, in virtual-desktop coordinates. The primary monitor
    /// starts at (0, 0); others can be negative.
    pub bounds: RECT,
    /// What is left once the taskbar and other appbars have taken their share
    /// — where a window should be placed.
    pub work_area: RECT,
    /// Effective dots per inch, `96` at 100% scaling.
    pub dpi: u32,
}

impl Monitor {
    /// The monitor the mouse pointer is on — where the user is looking, and
    /// so where a popup or an overlay belongs.
    pub fn from_cursor() -> Result<Self> {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point)? };
        Ok(Self::from_point(point))
    }

    /// The monitor nearest to `point` (never fails: an off-screen point maps
    /// to the closest monitor).
    pub fn from_point(point: POINT) -> Self {
        Self::describe(unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) })
    }

    /// The monitor a window is mostly on.
    pub fn from_window(hwnd: windows::Win32::Foundation::HWND) -> Self {
        Self::describe(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) })
    }

    /// The monitor of the window with the keyboard focus. `None` if that says
    /// nothing about where the user is: no window has it, it is hidden (a tray
    /// app's own window, after its menu), or it is the desktop, which spans
    /// every monitor.
    pub fn from_foreground() -> Option<Self> {
        let window = unsafe { GetForegroundWindow() };
        if window.is_invalid() || !unsafe { IsWindowVisible(window) }.as_bool() {
            return None;
        }

        let mut class = [0u16; 16];
        let length = unsafe { GetClassNameW(window, &mut class) };
        let class = String::from_utf16_lossy(&class[..length.max(0) as usize]);
        if matches!(class.as_str(), "Progman" | "WorkerW") {
            return None;
        }

        Some(Self::from_window(window))
    }

    pub fn primary() -> Self {
        Self::describe(unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) })
    }

    /// Scale factor: `1.0` at 96 DPI, `1.5` at 150%.
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / DEFAULT_DPI as f32
    }

    /// Converts a length expressed at 100% scaling to this monitor's pixels.
    pub fn scaled(&self, length: i32) -> i32 {
        (length * self.dpi as i32) / DEFAULT_DPI as i32
    }

    /// Centres a `size` rectangle in the work area.
    pub fn centre(&self, size: (i32, i32)) -> POINT {
        let area = self.work_area;
        POINT {
            x: area.left + ((area.right - area.left) - size.0) / 2,
            y: area.top + ((area.bottom - area.top) - size.1) / 2,
        }
    }

    /// Moves a `size` rectangle at `position` so it fits entirely in the work
    /// area — what a popup near the cursor needs when the cursor is at the
    /// edge of the screen.
    pub fn keep_inside(&self, position: POINT, size: (i32, i32)) -> POINT {
        let area = self.work_area;
        POINT {
            x: position.x.min(area.right - size.0).max(area.left),
            y: position.y.min(area.bottom - size.1).max(area.top),
        }
    }

    fn describe(handle: HMONITOR) -> Self {
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };

        // A monitor handle is only invalid if the monitor vanished between the
        // two calls; a zeroed rectangle is a harmless answer to that race.
        let _ = unsafe { GetMonitorInfoW(handle, &mut info) };

        let mut dpi = (DEFAULT_DPI, DEFAULT_DPI);
        let _ = unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi.0, &mut dpi.1) };

        Self {
            bounds: info.rcMonitor,
            work_area: info.rcWork,
            // Horizontal and vertical DPI differ only on exotic hardware.
            dpi: dpi.0,
        }
    }
}

/// Current mouse position, in virtual-desktop coordinates.
pub fn cursor_position() -> Result<POINT> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point)? };
    Ok(point)
}
