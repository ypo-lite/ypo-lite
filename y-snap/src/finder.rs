//! The mouse finder: plays one [`Style`] around the pointer.
//!
//! The overlay window exists only while an animation runs: [`Player::play`]
//! creates it and arms a `WM_TIMER` on the owner window, [`Player::tick`]
//! draws one frame per timer message, and the window and the timer are gone
//! when the animation ends. Nothing runs, and nothing is allocated, the rest
//! of the time.
//!
//! The pointer is read again on every frame, so the effect follows it, onto
//! another monitor too: a surface is fixed to one monitor (and one scale), so
//! crossing to another builds a new one there.
//!
//! The drawing is in [`crate::finder_overlay`].

use std::time::Instant;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, SetTimer, SetWindowDisplayAffinity, WINDOW_DISPLAY_AFFINITY,
};
use y_core::monitor::{self, Monitor};
use y_core::overlay::{Color, Overlay, OverlaySpec};

use crate::config::{Look, Shape, Style};
use crate::finder_overlay::{self as paint, Capturer, Frame};

/// The `WM_TIMER` id of the animation, on the owner window.
pub const TIMER_ID: usize = 4;

/// About 60 frames a second; Windows rounds timers to its own tick anyway.
const FRAME_MS: u32 = 16;

/// `WDA_EXCLUDEFROMCAPTURE`: the effect never shows up in a screen grab, so
/// what is read from the screen is always what is behind it, and a screen
/// recording does not see it. (The `windows` crate has no named constant.)
const EXCLUDE_FROM_CAPTURE: WINDOW_DISPLAY_AFFINITY = WINDOW_DISPLAY_AFFINITY(0x11);

/// The persistent shapes (halo, negative, contrast) stay this many times the
/// style's duration, but never longer than [`PERSISTENT_MAX_MS`].
const PERSISTENT_FACTOR: u32 = 4;
const PERSISTENT_MAX_MS: u32 = 10_000;

/// How long a look plays, in milliseconds.
pub fn play_time_ms(look: &Look) -> u32 {
    if look.shape.is_persistent() {
        (look.duration_ms * PERSISTENT_FACTOR).min(PERSISTENT_MAX_MS)
    } else {
        look.duration_ms
    }
}

pub struct Player {
    owner: HWND,
    run: Option<Run>,
}

/// One animation in progress.
struct Run {
    look: Look,
    overlay: Overlay,
    size: (i32, i32),
    /// Base radius in physical pixels, and the surface monitor's scale.
    radius: f32,
    scale: f32,
    monitor: Monitor,
    monitors: Vec<Monitor>,
    /// The base colour is `look.color`; this is what is drawn, which follows
    /// the backdrop when `auto-color` is on.
    color: Color,
    lum: f32,
    backdrop: Vec<u32>,
    capture: Option<Capturer>,
    sampler: Option<Capturer>,
    start: Instant,
    total_ms: f32,
    /// Frames shown on this surface, so that a monitor-sized one is pushed in
    /// full once and only refreshed after.
    shown: u32,
    /// With `all-monitors`, a surface on each of the other monitors, drawn
    /// with the same frame. Rebuilt when the pointer changes monitor, as the
    /// "other" ones are then different.
    extras: Vec<Extra>,
}

/// A surface on a monitor the pointer is not on.
struct Extra {
    overlay: Overlay,
    size: (i32, i32),
    scale: f32,
    origin: POINT,
    shown: u32,
}

/// One surface on every monitor but `current`, when the look asks for it and
/// its shape can.
fn extras(look: &Look, current: &Monitor, monitors: &[Monitor]) -> Vec<Extra> {
    if !docked(look) {
        return Vec::new();
    }
    monitors
        .iter()
        .filter(|m| m.bounds != current.bounds)
        .filter_map(|m| {
            let (overlay, size, _, scale) = surface(look, m, monitors)?;
            Some(Extra {
                overlay,
                size,
                scale,
                origin: paint::map_panel_origin(m.bounds, size, scale),
                shown: 0,
            })
        })
        .collect()
}

impl Player {
    pub fn new(owner: HWND) -> Self {
        Self { owner, run: None }
    }

    /// Plays `style`, replacing what plays now.
    pub fn play(&mut self, style: &Style) {
        self.stop();
        let Ok(monitor) = Monitor::from_cursor() else {
            return;
        };
        let mut monitors = Monitor::all();
        monitor::sort_spatially(&mut monitors);

        let look = style.look;
        let Some((overlay, size, radius, scale)) = surface(&look, &monitor, &monitors) else {
            return;
        };
        let extras = extras(&look, &monitor, &monitors);
        self.run = Some(Run {
            look,
            extras,
            overlay,
            size,
            radius,
            scale,
            monitor,
            monitors,
            color: look.color,
            lum: 0.5,
            backdrop: Vec::new(),
            capture: None,
            sampler: None,
            start: Instant::now(),
            total_ms: play_time_ms(&look) as f32,
            shown: 0,
        });
        let _ = unsafe { SetTimer(Some(self.owner), TIMER_ID, FRAME_MS, None) };
        self.tick();
    }

    /// One frame; the end of the animation when its time is up.
    pub fn tick(&mut self) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        if run.start.elapsed().as_secs_f32() * 1000.0 >= run.total_ms {
            self.stop();
            return;
        }
        run.frame();
    }

    /// Ends the animation: the timer, then the window.
    pub fn stop(&mut self) {
        if self.run.take().is_some() {
            let _ = unsafe { KillTimer(Some(self.owner), TIMER_ID) };
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Whether the look shows on every monitor, each with a small surface holding
/// only the map panel: no border around the monitor, so little memory.
fn docked(look: &Look) -> bool {
    look.all_monitors && look.shape.spreads()
}

/// A surface for `look` on `monitor`: small around the pointer, docked to the
/// monitor's map position, or covering the monitor. Returns the radius and
/// scale it was sized for.
fn surface(
    look: &Look,
    monitor: &Monitor,
    all: &[Monitor],
) -> Option<(Overlay, (i32, i32), f32, f32)> {
    let scale = monitor.dpi as f32 / 96.0;
    let radius = look.size as f32 * scale;
    let size = if docked(look) {
        paint::map_panel_size(all, scale)
    } else if paint::is_small(look.shape) {
        let side = (radius * 2.6).ceil() as i32;
        (side, side)
    } else {
        (
            monitor.bounds.right - monitor.bounds.left,
            monitor.bounds.bottom - monitor.bounds.top,
        )
    };
    let overlay = Overlay::new(&OverlaySpec::new("YpoFinder", size)).ok()?;
    let _ = unsafe { SetWindowDisplayAffinity(overlay.hwnd(), EXCLUDE_FROM_CAPTURE) };
    Some((overlay, size, radius, scale))
}

impl Run {
    fn frame(&mut self) {
        let pointer = monitor::cursor_position().unwrap_or_default();

        // The pointer went to another monitor (or another scale): the surface
        // has to follow it, so build a new one there.
        let current = Monitor::from_point(pointer);
        if (current.bounds != self.monitor.bounds || current.dpi != self.monitor.dpi)
            && let Some((overlay, size, radius, scale)) =
                surface(&self.look, &current, &self.monitors)
        {
            self.overlay = overlay;
            self.size = size;
            self.radius = radius;
            self.scale = scale;
            self.monitor = current;
            self.extras = extras(&self.look, &self.monitor, &self.monitors);
            self.capture = None;
            self.shown = 0;
        }

        let small = paint::is_small(self.look.shape);
        let (w, h) = self.size;
        let docked = docked(&self.look);
        let origin = if docked {
            paint::map_panel_origin(self.monitor.bounds, self.size, self.scale)
        } else if small {
            POINT {
                x: pointer.x - w / 2,
                y: pointer.y - h / 2,
            }
        } else {
            POINT {
                x: self.monitor.bounds.left,
                y: self.monitor.bounds.top,
            }
        };
        let centre = ((pointer.x - origin.x) as f32, (pointer.y - origin.y) as f32);

        self.sense(small, pointer, origin);

        let secs = self.start.elapsed().as_secs_f32();
        let frame = Frame {
            shape: self.look.shape,
            radius: self.radius,
            scale: self.scale,
            color: self.color,
            dim: self.look.dim as f32 / 100.0,
            monitors: &self.monitors,
            bounds: self.monitor.bounds,
            docked,
            pointer,
            backdrop: &self.backdrop,
            lum: self.lum,
            t: secs * 1000.0 / self.total_ms,
            secs,
            total: self.total_ms / 1000.0,
        };
        let canvas = self.overlay.canvas_mut();
        canvas.clear(Color::TRANSPARENT);
        paint::draw(canvas.pixels_mut(), (w, h), centre, &frame);

        // The same map panel on the other monitors, at their own scale (see
        // `Shape::spreads`); the map does not use the pointer's position on
        // the surface.
        for extra in &mut self.extras {
            let other = Frame {
                scale: extra.scale,
                ..frame
            };
            let canvas = extra.overlay.canvas_mut();
            canvas.clear(Color::TRANSPARENT);
            paint::draw(canvas.pixels_mut(), extra.size, centre, &other);
            let _ = if extra.shown == 0 {
                extra.overlay.show_at(extra.origin)
            } else {
                extra.overlay.refresh()
            };
            extra.shown += 1;
        }

        // A small surface moves with the pointer, so it is placed every time;
        // a monitor-sized one stays put and is only refreshed.
        let _ = if small || self.shown == 0 {
            self.overlay.show_at(origin)
        } else {
            self.overlay.refresh()
        };
        self.shown += 1;
    }

    /// Reads what is behind the effect: the whole surface for the shapes that
    /// draw from it, and the brightness for `auto-color` and for the shapes
    /// whose outline depends on it.
    fn sense(&mut self, small: bool, pointer: POINT, origin: POINT) {
        let wants_backdrop = paint::reads_backdrop(self.look.shape);
        let wants_lum = wants_backdrop || self.look.auto_color || self.look.shape == Shape::Halo;
        if !wants_lum {
            return;
        }
        let (w, h) = self.size;

        if small {
            if self.capture.is_none() {
                self.capture = Capturer::new(w, h);
            }
            if let Some(capture) = self.capture.as_mut() {
                let pixels = capture.grab(origin.x, origin.y);
                self.backdrop.clear();
                self.backdrop.extend_from_slice(pixels);
            }
        }
        let target = if small {
            paint::average_luminance(&self.backdrop)
        } else {
            // A patch around the pointer is enough for a brightness.
            if self.sampler.is_none() {
                self.sampler = Capturer::new(48, 48);
            }
            self.sampler
                .as_mut()
                .map(|sampler| {
                    paint::average_luminance(sampler.grab(pointer.x - 24, pointer.y - 24))
                })
                .unwrap_or(0.5)
        };
        self.lum += (target - self.lum) * 0.3;
        if self.look.auto_color {
            self.color = paint::auto_color(self.look.color, self.lum);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Shape;

    #[test]
    fn persistent_shapes_play_longer_but_not_forever() {
        let mut look = Look {
            shape: Shape::Beacon,
            ..Look::default()
        };
        assert_eq!(play_time_ms(&look), 1000);
        look.shape = Shape::Halo;
        assert_eq!(play_time_ms(&look), 4000);
        look.duration_ms = 5000;
        assert_eq!(play_time_ms(&look), 10_000);
    }
}
