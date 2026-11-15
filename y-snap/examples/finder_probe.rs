//! THROWAWAY — Phase 0 probes of `docs/design/mouse-finder-plan.md`.
//! Delete this file when the plan's Phase 0 results are recorded.
//!
//! ```text
//! cargo run -p y-snap --example finder_probe -- keys     # Ctrl-tap timing
//! cargo run -p y-snap --example finder_probe -- shake    # shake signal
//! cargo run -p y-snap --example finder_probe -- cursor   # 4x cursor ghost
//! cargo run -p y-snap --example finder_probe -- guard    # "do not disturb" signals
//! ```
//!
//! Everything is also written to `finder-probe.log` in the temp directory
//! (the path is printed first), so a probe left running behind a game can be
//! read afterwards. Stop any probe with Ctrl+C in its console.
//!
//! The hooks only record raw events; formatting and the state machines run
//! from the loop below, so the probe itself cannot trip the 300 ms hook limit.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::fs::File;
use std::io::Write;
use std::sync::{Mutex, mpsc};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetObjectW, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Shell::SHQueryUserNotificationState;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CURSORINFO, DI_NORMAL, DispatchMessageW, DrawIconEx, GetClassNameW,
    GetClipCursor, GetCursorInfo, GetForegroundWindow, GetIconInfo, GetSystemMetrics, HC_ACTION,
    HHOOK, ICONINFO, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, PM_REMOVE, PeekMessageW,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SPI_GETMOUSESONAR, SetWindowsHookExW, SystemParametersInfoW, TranslateMessage,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SYSKEYDOWN, WM_SYSKEYUP, HICON, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};
use windows::core::BOOL;
use windows::Win32::Graphics::Gdi::{BitBlt, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SRCCOPY};
use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WINDOW_DISPLAY_AFFINITY};
use y_core::monitor::Monitor;
use y_core::overlay::{Color, Overlay, OverlaySpec};

// ----------------------------------------------------------------- logging

static LOG: Mutex<Option<File>> = Mutex::new(None);
static START: Mutex<Option<Instant>> = Mutex::new(None);

fn say(text: impl AsRef<str>) {
    let elapsed = START
        .lock()
        .unwrap()
        .map(|start| start.elapsed().as_millis())
        .unwrap_or(0);
    let line = format!("[{elapsed:>7} ms] {}", text.as_ref());
    println!("{line}");
    if let Some(file) = LOG.lock().unwrap().as_mut() {
        let _ = writeln!(file, "{line}");
    }
}

// ------------------------------------------------------------------ events

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum Event {
    Key { vk: u32, down: bool, injected: bool, time: u32 },
    Move { x: i32, y: i32, injected: bool, time: u32 },
    Button { name: &'static str, down: bool, injected: bool, time: u32 },
}

thread_local! {
    static EVENTS: RefCell<Vec<Event>> = RefCell::new(Vec::with_capacity(8192));
}

fn push(event: Event) {
    EVENTS.with(|events| events.borrow_mut().push(event));
}

fn take_events() -> Vec<Event> {
    EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()))
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let down = match wparam.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(true),
            WM_KEYUP | WM_SYSKEYUP => Some(false),
            _ => None,
        };
        if let Some(down) = down {
            push(Event::Key {
                vk: info.vkCode,
                down,
                injected: info.flags.0 & 0x10 != 0, // LLKHF_INJECTED
                time: info.time,
            });
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let injected = info.flags & 0x01 != 0; // LLMHF_INJECTED
        let time = info.time;
        match wparam.0 as u32 {
            WM_MOUSEMOVE => push(Event::Move { x: info.pt.x, y: info.pt.y, injected, time }),
            WM_LBUTTONDOWN => push(Event::Button { name: "left", down: true, injected, time }),
            WM_LBUTTONUP => push(Event::Button { name: "left", down: false, injected, time }),
            WM_RBUTTONDOWN => push(Event::Button { name: "right", down: true, injected, time }),
            WM_RBUTTONUP => push(Event::Button { name: "right", down: false, injected, time }),
            WM_MBUTTONDOWN => push(Event::Button { name: "middle", down: true, injected, time }),
            WM_MBUTTONUP => push(Event::Button { name: "middle", down: false, injected, time }),
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn install(kind: windows::Win32::UI::WindowsAndMessaging::WINDOWS_HOOK_ID,
           procedure: unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT) -> HHOOK {
    let instance = unsafe { GetModuleHandleW(None) }.expect("module handle");
    unsafe { SetWindowsHookExW(kind, Some(procedure), Some(instance.into()), 0) }
        .expect("installing the hook")
}

/// Pumps this thread's messages (hooks and windows need it); `true` while the
/// run should go on.
fn pump() {
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        let _ = unsafe { TranslateMessage(&message) };
        unsafe { DispatchMessageW(&message) };
    }
}

// ------------------------------------------------------------ probe 1: keys

/// A tap is a press shorter than this (ms)…
const TAP_MAX_HOLD: u32 = 200;
/// …and two taps are a double when their presses are at most this far apart.
const DOUBLE_MAX_GAP: u32 = 400;

const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;

fn keys() {
    say("KEYS probe. Do, in turn: two quick Ctrl taps; two slow Ctrl taps; Ctrl+C (copy);");
    say("Ctrl tap, then a letter, then Ctrl tap; left Ctrl then right Ctrl; hold Ctrl 2 s.");
    say("Reported: every Ctrl edge, each clean tap, and a DOUBLE when two clean taps are close.");
    let _hook = install(WH_KEYBOARD_LL, keyboard_hook);

    let mut ctrl_down_at: Option<u32> = None;
    let mut dirty = false;
    let mut last_tap_down: Option<u32> = None;

    loop {
        pump();
        for event in take_events() {
            let Event::Key { vk, down, injected, time } = event else { continue };
            let is_ctrl = vk == VK_LCONTROL || vk == VK_RCONTROL;
            let side = if vk == VK_LCONTROL { "L" } else { "R" };
            let inj = if injected { " INJECTED" } else { "" };
            if is_ctrl {
                if down {
                    if ctrl_down_at.is_some() {
                        continue; // auto-repeat of a held Ctrl
                    }
                    ctrl_down_at = Some(time);
                    dirty = false;
                    say(format!("Ctrl{side} down{inj}"));
                } else if let Some(at) = ctrl_down_at.take() {
                    let held = time.wrapping_sub(at);
                    if dirty {
                        say(format!("Ctrl{side} up   held {held} ms — DIRTY (a chord), not a tap"));
                        last_tap_down = None;
                    } else if held > TAP_MAX_HOLD {
                        // A press held too long is not a tap, and it also
                        // forgets the previous one: "press, hold, press" is
                        // the false positive to avoid.
                        say(format!("Ctrl{side} up   held {held} ms — too long for a tap (max {TAP_MAX_HOLD})"));
                        last_tap_down = None;
                    } else {
                        say(format!("Ctrl{side} up   held {held} ms — clean TAP"));
                        match last_tap_down {
                            Some(previous) if at.wrapping_sub(previous) <= DOUBLE_MAX_GAP => {
                                let gap = at.wrapping_sub(previous);
                                say(format!("   >>> DOUBLE TAP: {gap} ms between the two presses"));
                                last_tap_down = None;
                            }
                            Some(previous) => {
                                say(format!(
                                    "   second tap too late ({} ms, max {DOUBLE_MAX_GAP}) — counts as a first tap",
                                    at.wrapping_sub(previous)
                                ));
                                last_tap_down = Some(at);
                            }
                            None => last_tap_down = Some(at),
                        }
                    }
                }
            } else if down && ctrl_down_at.is_some() {
                dirty = true;
                say(format!("   other key {vk:#04x} while Ctrl held{inj}"));
            } else if down {
                // Another key between two taps cancels the pending double.
                if last_tap_down.is_some() {
                    say(format!("   key {vk:#04x} between taps — pending tap forgotten"));
                    last_tap_down = None;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ----------------------------------------------------------- probe 2: shake

static MARK: AtomicU32 = AtomicU32::new(0);

/// Every tweakable value of the shake detector. Override on the command line:
/// `finder_probe shake speed=1500 segment=50 ctrl-speed=800`.
struct Params {
    /// Detection window (ms).
    window_ms: u32,
    /// Travel back from the last extreme that counts as a reversal (px).
    segment: i32,
    /// Mean pointer speed below which nothing is a shake (px/s).
    speed: f32,
    /// Reversals needed on the main axis for a `horizontal` shake.
    reversals: u32,
    /// Main axis travel / other axis travel, for horizontal and vertical.
    dominance: f32,
    /// Smooth turning needed for a `circle` (degrees).
    circle: f32,
    /// After a detection: ignore everything for this long (ms).
    cooldown: u32,
    /// The relaxed profile while Ctrl is held: speed and reversals. Ctrl is
    /// already "I am looking for something", so the bar is lower; the key is
    /// only read, never swallowed or cancelled.
    ctrl_speed: f32,
    ctrl_reversals: u32,
}

static PARAMS: std::sync::OnceLock<Params> = std::sync::OnceLock::new();

fn p() -> &'static Params {
    PARAMS.get().expect("parameters are set in main")
}

fn parse_params(args: impl Iterator<Item = String>) -> Params {
    let mut params = Params {
        window_ms: 700,
        segment: 60,
        speed: 1800.0,
        reversals: 3,
        dominance: 1.5,
        circle: 450.0,
        cooldown: 1000,
        ctrl_speed: 900.0,
        ctrl_reversals: 2,
    };
    for arg in args {
        let Some((key, value)) = arg.split_once('=') else {
            println!("ignored argument {arg:?} (expected name=value)");
            continue;
        };
        let number = value.parse::<f32>();
        match (key, number) {
            ("window", Ok(v)) => params.window_ms = v as u32,
            ("segment", Ok(v)) => params.segment = v as i32,
            ("speed", Ok(v)) => params.speed = v,
            ("reversals", Ok(v)) => params.reversals = v as u32,
            ("dominance", Ok(v)) => params.dominance = v,
            ("circle", Ok(v)) => params.circle = v,
            ("cooldown", Ok(v)) => params.cooldown = v as u32,
            ("ctrl-speed", Ok(v)) => params.ctrl_speed = v,
            ("ctrl-reversals", Ok(v)) => params.ctrl_reversals = v as u32,
            _ => println!("ignored argument {arg:?}"),
        }
    }
    params
}

/// What the shape of the path says, beyond axis reversals.
struct Shape {
    /// Signed turning (radians) summed over *smooth* corners only — a circle
    /// is ±2π. A back-and-forth reversal is a sharp corner and is not added:
    /// it would otherwise accumulate noise into a fake "circle".
    smooth_turn: f32,
    /// Corners sharper than `SHARP` (a reversal of direction).
    sharp: u32,
    /// Distance travelled along each axis (px).
    path_x: f32,
    path_y: f32,
}

/// A corner turning more than this is a reversal, not a curve.
const SHARP: f32 = 120.0 * std::f32::consts::PI / 180.0;

/// The path is decimated to 20 px steps first so that jitter does not count.
fn shape(samples: &VecDeque<(i32, i32, u32)>) -> Shape {
    let mut points: Vec<(f32, f32)> = Vec::new();
    for s in samples {
        let p = (s.0 as f32, s.1 as f32);
        if points.last().is_none_or(|l| ((p.0 - l.0).powi(2) + (p.1 - l.1).powi(2)).sqrt() >= 20.0) {
            points.push(p);
        }
    }
    let mut result = Shape { smooth_turn: 0.0, sharp: 0, path_x: 0.0, path_y: 0.0 };
    let mut previous: Option<f32> = None;
    for pair in points.windows(2) {
        result.path_x += (pair[1].0 - pair[0].0).abs();
        result.path_y += (pair[1].1 - pair[0].1).abs();
        let angle = (pair[1].1 - pair[0].1).atan2(pair[1].0 - pair[0].0);
        if let Some(before) = previous {
            let mut delta = angle - before;
            while delta > std::f32::consts::PI {
                delta -= 2.0 * std::f32::consts::PI;
            }
            while delta < -std::f32::consts::PI {
                delta += 2.0 * std::f32::consts::PI;
            }
            if delta.abs() >= SHARP {
                result.sharp += 1;
            } else {
                result.smooth_turn += delta;
            }
        }
        previous = Some(angle);
    }
    result
}

/// The kind of shake, if it is one.
///
/// * `circle`: many gentle turns in one direction, almost no reversals.
/// * `horizontal`: x reverses at least 3 times and x dominates the travel —
///   y may wobble freely, as a hand's does.
/// * `vertical`: the same on y (hard to do with a mouse; kept to see if it
///   ever happens).
fn classify(rev_x: u32, rev_y: u32, shape: &Shape, speed: f32, ctrl: bool) -> Option<&'static str> {
    let (min_speed, min_rev) = if ctrl {
        (p().ctrl_speed, p().ctrl_reversals)
    } else {
        (p().speed, p().reversals)
    };
    if speed < min_speed {
        return None;
    }
    if shape.smooth_turn.abs() >= p().circle.to_radians() && shape.sharp <= 1 {
        return Some("circle");
    }
    if rev_x >= min_rev && shape.path_x >= p().dominance * shape.path_y {
        return Some("horizontal");
    }
    if rev_y >= min_rev && shape.path_y >= p().dominance * shape.path_x {
        return Some("vertical");
    }
    None
}


/// Direction reversals along one axis: a reversal counts once the pointer has
/// travelled `p().segment` px back from the last extreme.
fn reversals(values: impl Iterator<Item = i32>) -> u32 {
    let mut direction = 0;
    let mut extreme = None::<i32>;
    let mut count = 0;
    for value in values {
        let Some(e) = extreme else {
            extreme = Some(value);
            continue;
        };
        match direction {
            0 => {
                if value - e >= p().segment {
                    direction = 1;
                    extreme = Some(value);
                } else if e - value >= p().segment {
                    direction = -1;
                    extreme = Some(value);
                }
            }
            1 => {
                if value > e {
                    extreme = Some(value);
                } else if e - value >= p().segment {
                    direction = -1;
                    count += 1;
                    extreme = Some(value);
                }
            }
            _ => {
                if value < e {
                    extreme = Some(value);
                } else if value - e >= p().segment {
                    direction = 1;
                    count += 1;
                    extreme = Some(value);
                }
            }
        }
    }
    count
}

fn shake() {
    say("SHAKE probe. A section is everything between two presses of Enter in this console.");
    say("Suggested sections (press Enter after each, a summary is printed):");
    say("  1 normal pointing across the screen   2 deliberate shake, no button");
    say("  3 gentle small wiggle                 4 fast window drag (left button held)");
    say("  5 shake with Ctrl held                6 a big sweeping circle");
    std::thread::spawn(|| {
        let mut line = String::new();
        while std::io::stdin().read_line(&mut line).is_ok() {
            line.clear();
            MARK.fetch_add(1, Ordering::SeqCst);
        }
    });
    let _hook = install(WH_MOUSE_LL, mouse_hook);
    let _keys = install(WH_KEYBOARD_LL, keyboard_hook);
    let mut ctrl_held = false;
    let min_speed_of = |ctrl: bool| if ctrl { p().ctrl_speed } else { p().speed };

    let mut samples: VecDeque<(i32, i32, u32)> = VecDeque::new();
    let mut buttons = 0u32;
    let mut section = 1;
    let mut seen_mark = 0;
    let (mut fires, mut blocked, mut max_rev, mut max_ratio) = (0u32, 0u32, 0u32, 0.0f32);
    let mut last_report = 0u32;

    loop {
        pump();
        for event in take_events() {
            match event {
                Event::Button { name, down, .. } => {
                    buttons = if down { buttons + 1 } else { buttons.saturating_sub(1) };
                    say(format!("   button {name} {}", if down { "down" } else { "up" }));
                }
                Event::Move { x, y, injected, time } => {
                    if injected {
                        continue;
                    }
                    samples.push_back((x, y, time));
                    while samples.front().is_some_and(|s| time.wrapping_sub(s.2) > p().window_ms) {
                        samples.pop_front();
                    }
                    let rev_x = reversals(samples.iter().map(|s| s.0));
                    let rev_y = reversals(samples.iter().map(|s| s.1));
                    let rev = rev_x.max(rev_y);
                    let mut path = 0.0f32;
                    for pair in samples.iter().collect::<Vec<_>>().windows(2) {
                        let (dx, dy) = ((pair[1].0 - pair[0].0) as f32, (pair[1].1 - pair[0].1) as f32);
                        path += (dx * dx + dy * dy).sqrt();
                    }
                    let (first, last) = (samples.front().unwrap(), samples.back().unwrap());
                    let net = (((last.0 - first.0).pow(2) + (last.1 - first.1).pow(2)) as f32).sqrt();
                    let ratio = path / net.max(1.0);
                    max_rev = max_rev.max(rev);
                    let duration = last.2.wrapping_sub(first.2).max(1) as f32;
                    let speed = path * 1000.0 / duration;
                    let shape = shape(&samples);
                    if time.wrapping_sub(last_report) < p().cooldown {
                        continue;
                    }
                    let metrics = format!(
                        "ctrl={ctrl_held} rev x/y={rev_x}/{rev_y} smooth-turn={:.0}deg sharp={} path x/y={:.0}/{:.0} speed={speed:.0}px/s net={net:.0}px ratio={ratio:.1}",
                        shape.smooth_turn.to_degrees(), shape.sharp, shape.path_x, shape.path_y
                    );
                    match classify(rev_x, rev_y, &shape, speed, ctrl_held) {
                        Some(kind) if buttons == 0 => {
                            fires += 1;
                            max_ratio = max_ratio.max(ratio);
                            last_report = time;
                            say(format!("   FIRE {kind:<10} {metrics}"));
                            samples.clear();
                        }
                        Some(kind) => {
                            blocked += 1;
                            last_report = time;
                            say(format!("   blocked (button down) {kind} {metrics}"));
                            samples.clear();
                        }
                        None if rev_x + rev_y >= 3 && speed >= min_speed_of(ctrl_held) * 0.6 => {
                            last_report = time;
                            say(format!("   near miss  {metrics}"));
                        }
                        None => {}
                    }
                }
                Event::Key { vk, down, .. } => {
                    if vk == VK_LCONTROL || vk == VK_RCONTROL {
                        ctrl_held = down;
                    }
                }
            }
        }
        let mark = MARK.load(Ordering::SeqCst);
        if mark != seen_mark {
            seen_mark = mark;
            say(format!(
                "== section {section}: fires={fires} blocked={blocked} max reversals={max_rev} max path/net ratio={max_ratio:.1}"
            ));
            section += 1;
            (fires, blocked, max_rev, max_ratio) = (0, 0, 0, 0.0);
            samples.clear();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ---------------------------------------------------------- probe 3: cursor

const SCALE: i32 = 4;
const CANVAS: i32 = 320;

struct Rendered {
    handle: isize,
    pixels: Vec<u32>,
    size: i32,
    hotspot: (i32, i32),
}

/// The cursor as premultiplied `0xAARRGGBB`, drawn twice (on black, on white)
/// to recover per-pixel alpha whatever kind of cursor it is.
fn render(handle: HICON) -> Option<Rendered> {
    let mut info = ICONINFO::default();
    unsafe { GetIconInfo(handle, &mut info) }.ok()?;
    let colour = !info.hbmColor.is_invalid();
    let mut bitmap = BITMAP::default();
    unsafe {
        GetObjectW(
            if colour { info.hbmColor } else { info.hbmMask }.into(),
            size_of::<BITMAP>() as i32,
            Some(&mut bitmap as *mut _ as *mut c_void),
        )
    };
    let native = bitmap.bmWidth;
    let native_height = if colour { bitmap.bmHeight } else { bitmap.bmHeight / 2 };
    if !info.hbmColor.is_invalid() {
        let _ = unsafe { DeleteObject(info.hbmColor.into()) };
    }
    if !info.hbmMask.is_invalid() {
        let _ = unsafe { DeleteObject(info.hbmMask.into()) };
    }
    say(format!(
        "cursor {:#x}: {} image, native {native}x{native_height}, hotspot ({}, {})",
        handle.0 as usize,
        if colour { "colour" } else { "MONOCHROME (AND/XOR)" },
        info.xHotspot,
        info.yHotspot
    ));

    let size = native.max(native_height) * SCALE;
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let dc = unsafe { CreateCompatibleDC(None) };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let dib = unsafe { CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    let previous = unsafe { SelectObject(dc, dib.into()) };
    let count = (size * size) as usize;
    let surface = unsafe { std::slice::from_raw_parts_mut(bits as *mut u32, count) };

    let mut draw = |background: u32| {
        surface.fill(background);
        let _ = unsafe { DrawIconEx(dc, 0, 0, handle, size, size, 0, None, DI_NORMAL) };
        surface.to_vec()
    };
    let on_black = draw(0x0000_0000);
    let on_white = draw(0x00FF_FFFF);

    unsafe {
        SelectObject(dc, previous);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(dc);
    }

    let mut visible = 0usize;
    let pixels = on_black
        .iter()
        .zip(&on_white)
        .map(|(&b, &w)| {
            let channel = |value: u32, shift: u32| (value >> shift) & 0xFF;
            let transparency: u32 = [0, 8, 16]
                .iter()
                .map(|&s| channel(w, s).saturating_sub(channel(b, s)))
                .sum::<u32>()
                / 3;
            let alpha = 255u32.saturating_sub(transparency);
            if alpha > 0 {
                visible += 1;
            }
            let premultiplied = |shift: u32| channel(b, shift).min(alpha);
            (alpha << 24) | (premultiplied(16) << 16) | (premultiplied(8) << 8) | premultiplied(0)
        })
        .collect();
    say(format!(
        "   rendered at {size}x{size} ({SCALE}x): {visible} visible pixels of {count}"
    ));
    Some(Rendered {
        handle: handle.0 as isize,
        pixels,
        size,
        hotspot: (info.xHotspot as i32 * SCALE, info.yHotspot as i32 * SCALE),
    })
}

fn cursor() {
    say("CURSOR probe: a 4x copy of the pointer follows it for 90 s, with a red dot at the true");
    say("hotspot. Move over: text (I-beam), a link, a window edge (resize), busy, another monitor");
    say("with a different scale. Judge: sharpness, hotspot alignment, holes/inversions, lag.");
    let mut overlay =
        Overlay::new(&OverlaySpec::new("YpoFinderProbe", (CANVAS, CANVAS))).expect("overlay");
    let mut current: Option<Rendered> = None;
    let started = Instant::now();
    let mut shown = false;

    while started.elapsed() < Duration::from_secs(90) {
        pump();
        let mut info = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
        if unsafe { GetCursorInfo(&mut info) }.is_ok() {
            let visible = info.flags.0 & 1 != 0; // CURSOR_SHOWING
            if !visible || info.hCursor.is_invalid() {
                if shown {
                    overlay.hide();
                    shown = false;
                    say("pointer hidden by the system/app — overlay hidden");
                }
            } else {
                let handle = HICON(info.hCursor.0);
                if current.as_ref().is_none_or(|c| c.handle != handle.0 as isize) {
                    current = render(handle);
                }
                if let Some(cursor) = &current {
                    let monitor = Monitor::from_point(info.ptScreenPos);
                    let canvas = overlay.canvas_mut();
                    canvas.clear(Color::TRANSPARENT);
                    canvas.blit(
                        CANVAS / 2 - cursor.hotspot.0,
                        CANVAS / 2 - cursor.hotspot.1,
                        cursor.size,
                        cursor.size,
                        &cursor.pixels,
                    );
                    canvas.fill_rect(
                        RECT {
                            left: CANVAS / 2 - 1,
                            top: CANVAS / 2 - 1,
                            right: CANVAS / 2 + 2,
                            bottom: CANVAS / 2 + 2,
                        },
                        Color::rgb(255, 0, 0),
                    );
                    let _ = overlay.show_at(POINT {
                        x: info.ptScreenPos.x - CANVAS / 2,
                        y: info.ptScreenPos.y - CANVAS / 2,
                    });
                    shown = true;
                    let _ = monitor;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    say("cursor probe finished");
}

// ----------------------------------------------------------- probe 4: guard

fn guard() {
    say("GUARD probe: logs the do-not-disturb signals every second (and marks changes).");
    say("Leave it running while you switch to: a fullscreen game, a borderless-fullscreen game,");
    say("a fullscreen video, a presentation, a VM/remote-desktop window, a normal app. Read the");
    say("log file afterwards (path above) if the game hides this console.");

    let mut sonar = BOOL(0);
    let _ = unsafe {
        SystemParametersInfoW(
            SPI_GETMOUSESONAR,
            0,
            Some(&mut sonar as *mut _ as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    say(format!("Windows' own 'show pointer on Ctrl' (SPI_GETMOUSESONAR): {}", sonar.as_bool()));

    let mut previous = String::new();
    let mut ticks = 0u32;
    loop {
        let started = Instant::now();
        let quns = unsafe { SHQueryUserNotificationState() }
            .map(|state| format!("{state:?}"))
            .unwrap_or_else(|e| format!("error {e}"));
        let quns_cost = started.elapsed();

        let mut info = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
        let showing = unsafe { GetCursorInfo(&mut info) }.is_ok() && info.flags.0 & 1 != 0;

        let mut clip = RECT::default();
        let _ = unsafe { GetClipCursor(&mut clip) };
        let virtual_screen = RECT {
            left: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
            top: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
            right: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) },
            bottom: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) + GetSystemMetrics(SM_CYVIRTUALSCREEN) },
        };
        let confined = clip != virtual_screen;

        let window: HWND = unsafe { GetForegroundWindow() };
        let (class, exe, covers) = if window.is_invalid() {
            ("-".into(), "-".into(), false)
        } else {
            let mut buffer = [0u16; 64];
            let length = unsafe { GetClassNameW(window, &mut buffer) } as usize;
            let class = String::from_utf16_lossy(&buffer[..length]);
            let exe = y_core::process::executable_stem(y_core::process::window_process_id(window))
                .unwrap_or_else(|| "?".into());
            let mut rect = RECT::default();
            let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(window, &mut rect) };
            let bounds = Monitor::from_window(window).bounds;
            let covers = rect.left <= bounds.left
                && rect.top <= bounds.top
                && rect.right >= bounds.right
                && rect.bottom >= bounds.bottom;
            (class, exe, covers)
        };

        let line = format!(
            "QUNS={quns} ({} us) | cursor shown={showing} | clip {}..{} x {}..{} confined={confined} | fg {exe} [{class}] covers-monitor={covers}",
            quns_cost.as_micros(),
            clip.left, clip.right, clip.top, clip.bottom
        );
        let state = line.split(" us) ").nth(1).unwrap_or("").to_string();
        if state != previous || ticks.is_multiple_of(10) {
            say(if state != previous { format!("CHANGED: {line}") } else { line });
            previous = state;
        }
        ticks += 1;
        std::thread::sleep(Duration::from_secs(1));
    }
}

// --------------------------------------------------------- probe 5: styles

/// Tweakable values of the indicators. Override on the command line:
/// `finder_probe styles size=200 color=#00C8FF duration=1500 delay=1500`.
struct StyleParams {
    /// Wait between typing a number and the effect, to move the pointer away (ms).
    delay_ms: u64,
    /// Length of the effect (ms).
    duration_ms: f32,
    /// Base radius, in dips.
    size: f32,
    color: Color,
    /// Spotlight darkness, 0..1.
    dim: f32,
    /// How long the persistent halo stays (s), unless another command stops it.
    halo_seconds: f32,
    /// `color=auto`: the colour follows the brightness of the backdrop.
    auto: bool,
}

fn parse_style_params(args: impl Iterator<Item = String>) -> StyleParams {
    let mut params = StyleParams {
        delay_ms: 2000,
        duration_ms: 1200.0,
        size: 160.0,
        color: Color::rgb(0xFF, 0xB0, 0x00),
        dim: 0.6,
        halo_seconds: 15.0,
        auto: false,
    };
    for arg in args {
        let Some((key, value)) = arg.split_once('=') else {
            println!("ignored argument {arg:?}");
            continue;
        };
        match key {
            "delay" => params.delay_ms = value.parse().unwrap_or(params.delay_ms),
            "duration" => params.duration_ms = value.parse().unwrap_or(params.duration_ms),
            "size" => params.size = value.parse().unwrap_or(params.size),
            "color" if value == "auto" => params.auto = true,
            "color" => params.color = Color::parse(value).unwrap_or(params.color),
            "dim" => params.dim = value.parse().unwrap_or(params.dim),
            "halo-seconds" => params.halo_seconds = value.parse().unwrap_or(params.halo_seconds),
            _ => println!("ignored argument {arg:?}"),
        }
    }
    params
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ease_out(p: f32) -> f32 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(3)
}

/// Blends `color` at `alpha` over one premultiplied pixel.
fn blend(pixels: &mut [u32], index: usize, color: Color, alpha: f32) {
    let a = alpha.clamp(0.0, 1.0) * (color.a as f32 / 255.0);
    if a <= 0.0 {
        return;
    }
    let dst = pixels[index];
    let inv = 1.0 - a;
    let channel = |shift: u32, c: u8| {
        (((c as f32) * a + ((dst >> shift) & 0xFF) as f32 * inv).round() as u32).min(255)
    };
    let alpha_out = ((255.0 * a + ((dst >> 24) & 0xFF) as f32 * inv).round() as u32).min(255);
    pixels[index] = (alpha_out << 24)
        | (channel(16, color.r) << 16)
        | (channel(8, color.g) << 8)
        | channel(0, color.b);
}

/// A disc fading from full at the centre to nothing at `radius`.
#[allow(clippy::too_many_arguments)]
fn soft_disc(p: &mut [u32], w: i32, h: i32, cx: f32, cy: f32, radius: f32, color: Color, alpha: f32) {
    let (x0, x1) = (((cx - radius).floor() as i32).max(0), ((cx + radius).ceil() as i32).min(w - 1));
    let (y0, y1) = (((cy - radius).floor() as i32).max(0), ((cy + radius).ceil() as i32).min(h - 1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let coverage = (1.0 - d / radius).clamp(0.0, 1.0);
            blend(p, (y * w + x) as usize, color, alpha * coverage * coverage.sqrt());
        }
    }
}

/// An antialiased ring.
#[allow(clippy::too_many_arguments)]
fn ring(p: &mut [u32], w: i32, h: i32, cx: f32, cy: f32, radius: f32, thickness: f32, color: Color, alpha: f32) {
    let reach = radius + thickness;
    let (x0, x1) = (((cx - reach).floor() as i32).max(0), ((cx + reach).ceil() as i32).min(w - 1));
    let (y0, y1) = (((cy - reach).floor() as i32).max(0), ((cy + reach).ceil() as i32).min(h - 1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let coverage = (thickness / 2.0 + 0.5 - (d - radius).abs()).clamp(0.0, 1.0);
            if coverage > 0.0 {
                blend(p, (y * w + x) as usize, color, alpha * coverage);
            }
        }
    }
}

fn fill_rect_blend(p: &mut [u32], w: i32, h: i32, rect: RECT, color: Color, alpha: f32) {
    for y in rect.top.max(0)..rect.bottom.min(h) {
        for x in rect.left.max(0)..rect.right.min(w) {
            blend(p, (y * w + x) as usize, color, alpha);
        }
    }
}

struct Ctx {
    radius: f32,
    scale: f32,
    color: Color,
    dim: f32,
    monitors: Vec<Monitor>,
    bounds: RECT,
    pointer: POINT,
    /// The screen behind a small surface, same size as it (some effects only).
    backdrop: Vec<u32>,
    /// Smoothed brightness of what is behind the pointer, 0..1.
    lum: f32,
}

/// One frame. `t` is the progress 0..1, `secs` the elapsed seconds.
#[allow(clippy::too_many_arguments)]
fn draw(style: &str, p: &mut [u32], w: i32, h: i32, (cx, cy): (f32, f32), t: f32, secs: f32, c: &Ctx) {
    let fade = smoothstep(0.0, 0.1, t) * (1.0 - smoothstep(0.78, 1.0, t));
    let r = c.radius;
    match style {
        // Rings converging on the pointer over a soft halo.
        "beacon" => {
            soft_disc(p, w, h, cx, cy, 0.6 * r, c.color, 0.40 * fade);
            for i in 0..3 {
                let q = t * 1.5 - i as f32 * 0.2;
                if (0.0..=1.0).contains(&q) {
                    let radius = r * (1.0 - ease_out(q) * 0.85);
                    let alpha = smoothstep(0.0, 0.15, q) * (1.0 - 0.5 * q) * fade;
                    ring(p, w, h, cx, cy, radius, 6.0 * c.scale, c.color, alpha);
                }
            }
        }
        // The screen dims; a clear circle shrinks onto the pointer.
        "spotlight" => {
            let dim = c.dim * fade;
            p.fill(((dim * 255.0) as u32) << 24);
            let hole = r * (0.9 + 3.0 * (1.0 - ease_out(t / 0.35)));
            let soft = 0.3 * r;
            let reach = hole + soft;
            for y in ((cy - reach).floor() as i32).max(0)..=((cy + reach).ceil() as i32).min(h - 1) {
                for x in ((cx - reach).floor() as i32).max(0)..=((cx + reach).ceil() as i32).min(w - 1) {
                    let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                    let alpha = dim * smoothstep(hole, hole + soft, d);
                    p[(y * w + x) as usize] = ((alpha * 255.0) as u32) << 24;
                }
            }
        }
        // Lines across the whole monitor, with a gap and a ring at the pointer.
        "crosshair" => {
            let grow = ease_out(t / 0.25);
            let extent = (grow * w.max(h) as f32) as i32;
            let gap = (0.6 * r) as i32;
            let thick = ((3.0 * c.scale) as i32).max(1);
            let alpha = 0.65 * fade;
            let (ix, iy) = (cx as i32, cy as i32);
            for y in (iy - thick / 2).max(0)..(iy - thick / 2 + thick).min(h) {
                for x in 0..w {
                    let dx = (x - ix).abs();
                    if dx >= gap && dx <= extent {
                        blend(p, (y * w + x) as usize, c.color, alpha);
                    }
                }
            }
            for x in (ix - thick / 2).max(0)..(ix - thick / 2 + thick).min(w) {
                for y in 0..h {
                    let dy = (y - iy).abs();
                    if dy >= gap && dy <= extent {
                        blend(p, (y * w + x) as usize, c.color, alpha);
                    }
                }
            }
            ring(p, w, h, cx, cy, gap as f32, 3.0 * c.scale, c.color, 0.9 * fade);
        }
        // Which monitor: a pulsing border, and a map of all the monitors.
        "monitor" => {
            let pulse = 0.65 + 0.35 * (secs * std::f32::consts::TAU * 2.0).cos();
            let b = (10.0 * c.scale) as i32;
            let alpha = fade * pulse;
            for rect in [
                RECT { left: 0, top: 0, right: w, bottom: b },
                RECT { left: 0, top: h - b, right: w, bottom: h },
                RECT { left: 0, top: b, right: b, bottom: h - b },
                RECT { left: w - b, top: b, right: w, bottom: h - b },
            ] {
                fill_rect_blend(p, w, h, rect, c.color, alpha);
            }

            let union = c.monitors.iter().fold(
                RECT { left: i32::MAX, top: i32::MAX, right: i32::MIN, bottom: i32::MIN },
                |u, m| RECT {
                    left: u.left.min(m.bounds.left),
                    top: u.top.min(m.bounds.top),
                    right: u.right.max(m.bounds.right),
                    bottom: u.bottom.max(m.bounds.bottom),
                },
            );
            let (uw, uh) = ((union.right - union.left) as f32, (union.bottom - union.top) as f32);
            let (box_w, box_h) = (380.0 * c.scale, 200.0 * c.scale);
            let k = (box_w / uw).min(box_h / uh);
            let (map_w, map_h) = (uw * k, uh * k);
            let pad = 16.0 * c.scale;
            let (mx, my) = (w as f32 / 2.0 - map_w / 2.0, h as f32 * 0.22);
            let panel = RECT {
                left: (mx - pad) as i32,
                top: (my - pad) as i32,
                right: (mx + map_w + pad) as i32,
                bottom: (my + map_h + pad) as i32,
            };
            fill_rect_blend(p, w, h, panel, Color::rgb(20, 20, 20), 0.8 * fade);
            for m in &c.monitors {
                let rect = RECT {
                    left: (mx + (m.bounds.left - union.left) as f32 * k) as i32 + 2,
                    top: (my + (m.bounds.top - union.top) as f32 * k) as i32 + 2,
                    right: (mx + (m.bounds.right - union.left) as f32 * k) as i32 - 2,
                    bottom: (my + (m.bounds.bottom - union.top) as f32 * k) as i32 - 2,
                };
                let current = m.bounds.left == c.bounds.left && m.bounds.top == c.bounds.top;
                let (colour, strength) = if current { (c.color, 0.55) } else { (Color::WHITE, 0.14) };
                fill_rect_blend(p, w, h, rect, colour, strength * fade);
                let mut edge = |r: RECT| fill_rect_blend(p, w, h, r, Color::WHITE, 0.85 * fade);
                edge(RECT { left: rect.left, top: rect.top, right: rect.right, bottom: rect.top + 2 });
                edge(RECT { left: rect.left, top: rect.bottom - 2, right: rect.right, bottom: rect.bottom });
                edge(RECT { left: rect.left, top: rect.top, right: rect.left + 2, bottom: rect.bottom });
                edge(RECT { left: rect.right - 2, top: rect.top, right: rect.right, bottom: rect.bottom });
            }
            let (dot_x, dot_y) = (
                mx + (c.pointer.x - union.left) as f32 * k,
                my + (c.pointer.y - union.top) as f32 * k,
            );
            ring(p, w, h, dot_x, dot_y, 7.0 * c.scale, 3.0 * c.scale, Color::WHITE, fade);
            soft_disc(p, w, h, dot_x, dot_y, 6.0 * c.scale, Color::rgb(255, 40, 40), fade);
        }
        // A persistent ring and glow that follow the pointer.
        "halo" => {
            let fade = smoothstep(0.0, 0.3, secs);
            soft_disc(p, w, h, cx, cy, 0.55 * r, c.color, 0.35 * fade);
            ring(p, w, h, cx, cy, 0.5 * r, 3.0 * c.scale, c.color, 0.85 * fade);
        }
        // A halo whose colour follows the backdrop (see `play`), with a thin
        // outline of the opposite tone so it reads on mixed backgrounds too.
        "autohalo" => {
            let fade = smoothstep(0.0, 0.3, secs);
            soft_disc(p, w, h, cx, cy, 0.55 * r, c.color, 0.40 * fade);
            ring(p, w, h, cx, cy, 0.5 * r, 4.0 * c.scale, c.color, 0.95 * fade);
            ring(p, w, h, cx, cy, 0.5 * r + 3.5 * c.scale, 2.0 * c.scale, outline_for(c.lum), 0.8 * fade);
        }
        // A lens that shows the screen behind the pointer as a negative.
        "negative" => {
            let fade = smoothstep(0.0, 0.2, secs);
            let radius = 0.5 * r;
            let soft = 0.12 * r;
            if c.backdrop.len() >= (w * h) as usize {
                let reach = radius + 1.0;
                for y in ((cy - reach).floor() as i32).max(0)..=((cy + reach).ceil() as i32).min(h - 1) {
                    for x in ((cx - reach).floor() as i32).max(0)..=((cx + reach).ceil() as i32).min(w - 1) {
                        let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                        let coverage = 1.0 - smoothstep(radius - soft, radius, d);
                        if coverage > 0.0 {
                            let index = (y * w + x) as usize;
                            let b = c.backdrop[index];
                            let inverted = Color::rgb(
                                255 - ((b >> 16) & 0xFF) as u8,
                                255 - ((b >> 8) & 0xFF) as u8,
                                255 - (b & 0xFF) as u8,
                            );
                            blend(p, index, inverted, coverage * fade);
                        }
                    }
                }
            }
            ring(p, w, h, cx, cy, radius, 2.0 * c.scale, outline_for(c.lum), 0.7 * fade);
        }
        // A ring painted black or white pixel by pixel, whichever contrasts
        // with the pixel behind it.
        "contrast" => {
            let fade = smoothstep(0.0, 0.2, secs);
            let radius = 0.5 * r;
            let thickness = 8.0 * c.scale;
            if c.backdrop.len() >= (w * h) as usize {
                let reach = radius + thickness;
                for y in ((cy - reach).floor() as i32).max(0)..=((cy + reach).ceil() as i32).min(h - 1) {
                    for x in ((cx - reach).floor() as i32).max(0)..=((cx + reach).ceil() as i32).min(w - 1) {
                        let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                        let coverage = (thickness / 2.0 + 0.5 - (d - radius).abs()).clamp(0.0, 1.0);
                        if coverage > 0.0 {
                            let index = (y * w + x) as usize;
                            let colour = if luminance(c.backdrop[index]) > 0.5 { Color::BLACK } else { Color::WHITE };
                            blend(p, index, colour, 0.92 * coverage * fade);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// The "do not disturb" signals, on one line (see `guard`).
fn guard_snapshot() -> String {
    let quns = unsafe { SHQueryUserNotificationState() }
        .map(|state| format!("{state:?}"))
        .unwrap_or_else(|e| format!("error {e}"));
    let mut info = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
    let showing = unsafe { GetCursorInfo(&mut info) }.is_ok() && info.flags.0 & 1 != 0;
    let mut clip = RECT::default();
    let _ = unsafe { GetClipCursor(&mut clip) };
    let virtual_screen = RECT {
        left: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
        top: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
        right: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) },
        bottom: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) + GetSystemMetrics(SM_CYVIRTUALSCREEN) },
    };
    let window: HWND = unsafe { GetForegroundWindow() };
    let (class, exe, covers) = if window.is_invalid() {
        ("-".into(), "-".into(), false)
    } else {
        let mut buffer = [0u16; 64];
        let length = unsafe { GetClassNameW(window, &mut buffer) } as usize;
        let exe = y_core::process::executable_stem(y_core::process::window_process_id(window))
            .unwrap_or_else(|| "?".into());
        let mut rect = RECT::default();
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(window, &mut rect) };
        let bounds = Monitor::from_window(window).bounds;
        let covers = rect.left <= bounds.left
            && rect.top <= bounds.top
            && rect.right >= bounds.right
            && rect.bottom >= bounds.bottom;
        (String::from_utf16_lossy(&buffer[..length]), exe, covers)
    };
    format!(
        "guard: QUNS={quns} | cursor shown={showing} | confined={} | fg {exe} [{class}] covers-monitor={covers}",
        clip != virtual_screen
    )
}

/// A reusable copy of a screen region. Layered windows (ours) are not part of
/// a plain `BitBlt` from the screen, and the overlays also opt out of capture,
/// so what comes back is what is *behind* the effect.
struct Capturer {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Capturer {
    fn new(w: i32, h: i32) -> Option<Self> {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let dc = unsafe { CreateCompatibleDC(None) };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap =
            unsafe { CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
        if bits.is_null() {
            return None;
        }
        let previous = unsafe { SelectObject(dc, bitmap.into()) };
        Some(Self { dc, bitmap, previous, bits: bits as *mut u32, w, h })
    }

    /// The `w`x`h` region of the screen whose top left is `(x, y)`, top row first.
    fn grab(&mut self, x: i32, y: i32) -> &[u32] {
        unsafe {
            let screen = GetDC(None);
            let _ = BitBlt(self.dc, 0, 0, self.w, self.h, Some(screen), x, y, SRCCOPY);
            ReleaseDC(None, screen);
            std::slice::from_raw_parts(self.bits, (self.w * self.h) as usize)
        }
    }
}

impl Drop for Capturer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

fn luminance(pixel: u32) -> f32 {
    let (r, g, b) = ((pixel >> 16) & 0xFF, (pixel >> 8) & 0xFF, pixel & 0xFF);
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
}

/// Average luminance of a pixel buffer (sampled, not exhaustive).
fn average_luminance(pixels: &[u32]) -> f32 {
    if pixels.is_empty() {
        return 0.5;
    }
    let step = (pixels.len() / 2048).max(1);
    let (mut sum, mut count) = (0.0, 0u32);
    for pixel in pixels.iter().step_by(step) {
        sum += luminance(*pixel);
        count += 1;
    }
    sum / count as f32
}

/// The configured colour, darkened on a light backdrop and lightened on a
/// dark one, so it never melts into what is behind it. `lum` is 0 (black)..1.
fn auto_color(base: Color, lum: f32) -> Color {
    let light = smoothstep(0.35, 0.65, lum);
    let mix = |c: u8| {
        let for_dark = c as f32 + (255.0 - c as f32) * 0.35;
        let for_light = c as f32 * 0.3;
        (for_dark + (for_light - for_dark) * light).round() as u8
    };
    Color { r: mix(base.r), g: mix(base.g), b: mix(base.b), a: base.a }
}

/// Black or white, whichever separates from the ring drawn for `lum`.
fn outline_for(lum: f32) -> Color {
    if lum > 0.5 { Color::WHITE } else { Color::BLACK }
}

fn make_surface(
    small: bool,
    monitor: &Monitor,
    params: &StyleParams,
) -> Option<(Overlay, i32, i32, f32, f32)> {
    let scale = monitor.dpi as f32 / 96.0;
    let radius = params.size * scale;
    let (w, h) = if small {
        let side = (radius * 2.6).ceil() as i32;
        (side, side)
    } else {
        (monitor.bounds.right - monitor.bounds.left, monitor.bounds.bottom - monitor.bounds.top)
    };
    match Overlay::new(&OverlaySpec::new("YpoFinderStyles", (w, h))) {
        Ok(overlay) => {
            // WDA_EXCLUDEFROMCAPTURE: our own pixels never come back in a grab.
            let _ = unsafe { SetWindowDisplayAffinity(overlay.hwnd(), WINDOW_DISPLAY_AFFINITY(0x11)) };
            Some((overlay, w, h, radius, scale))
        }
        Err(error) => {
            say(format!("cannot create the overlay: {error}"));
            None
        }
    }
}

fn play(style: &str, params: &StyleParams, commands: &mpsc::Receiver<String>) {
    let Ok(mut monitor) = Monitor::from_cursor() else { return };
    let small = matches!(style, "beacon" | "halo" | "autohalo" | "negative" | "contrast");
    let persistent = matches!(style, "halo" | "autohalo" | "negative" | "contrast");
    let wants_backdrop = matches!(style, "negative" | "contrast");
    let auto = params.auto || style == "autohalo";
    let mut monitors = Monitor::all();
    y_core::monitor::sort_spatially(&mut monitors);

    let Some((mut overlay, mut w, mut h, radius, scale)) = make_surface(small, &monitor, params) else {
        return;
    };
    let mut ctx = Ctx {
        radius,
        scale,
        color: params.color,
        dim: params.dim,
        monitors,
        bounds: monitor.bounds,
        pointer: POINT::default(),
        backdrop: Vec::new(),
        lum: 0.5,
    };
    let duration = if persistent { params.halo_seconds * 1000.0 } else { params.duration_ms };

    let mut capture: Option<Capturer> = None;
    let mut sampler: Option<Capturer> = None;
    let start = Instant::now();
    let (mut frames, mut total, mut worst) = (0u32, 0.0f32, 0.0f32);
    let mut surface_frames = 0u32;
    let mut switches = 0u32;
    loop {
        pump();
        let elapsed = start.elapsed().as_secs_f32();
        let t = elapsed * 1000.0 / duration;
        if t >= 1.0 || commands.try_recv().is_ok() {
            break;
        }
        let frame_start = Instant::now();
        let pointer = y_core::monitor::cursor_position().unwrap_or_default();
        ctx.pointer = pointer;

        // The pointer went to another monitor (or another scale): the surface
        // has to follow it, so build a new one there.
        let current = Monitor::from_point(pointer);
        if (current.bounds != monitor.bounds || current.dpi != monitor.dpi)
            && let Some(surface) = make_surface(small, &current, params)
        {
                (overlay, w, h, ctx.radius, ctx.scale) = surface;
                monitor = current;
                ctx.bounds = monitor.bounds;
                surface_frames = 0;
                capture = None;
                switches += 1;
        }

        let origin = if small {
            POINT { x: pointer.x - w / 2, y: pointer.y - h / 2 }
        } else {
            POINT { x: monitor.bounds.left, y: monitor.bounds.top }
        };
        let centre = ((pointer.x - origin.x) as f32, (pointer.y - origin.y) as f32);

        // What is behind the effect: the whole surface for the effects that
        // draw from it, a small patch around the pointer when only the
        // average brightness is wanted.
        if small && (wants_backdrop || auto) {
            if capture.is_none() {
                capture = Capturer::new(w, h);
            }
            if let Some(capture) = capture.as_mut() {
                let pixels = capture.grab(origin.x, origin.y);
                ctx.backdrop.clear();
                ctx.backdrop.extend_from_slice(pixels);
            }
        }
        if auto {
            let target = if small {
                average_luminance(&ctx.backdrop)
            } else {
                if sampler.is_none() {
                    sampler = Capturer::new(48, 48);
                }
                sampler
                    .as_mut()
                    .map(|s| average_luminance(s.grab(pointer.x - 24, pointer.y - 24)))
                    .unwrap_or(0.5)
            };
            ctx.lum += (target - ctx.lum) * 0.3;
            ctx.color = auto_color(params.color, ctx.lum);
        }

        {
            let canvas = overlay.canvas_mut();
            canvas.clear(Color::TRANSPARENT);
            draw(style, canvas.pixels_mut(), w, h, centre, t, elapsed, &ctx);
        }
        let _ = if small || surface_frames == 0 { overlay.show_at(origin) } else { overlay.refresh() };
        surface_frames += 1;

        let cost = frame_start.elapsed().as_secs_f32() * 1000.0;
        frames += 1;
        total += cost;
        worst = worst.max(cost);
        if cost < 16.0 {
            std::thread::sleep(Duration::from_millis((16.0 - cost) as u64));
        }
    }
    say(format!(
        "{style}: {frames} frames, {:.1} ms/frame average, {worst:.1} ms worst, {switches} monitor change(s)",
        total / frames.max(1) as f32
    ));
}

fn styles() {
    let params = parse_style_params(std::env::args().skip(2));
    say("STYLES probe — use `cargo run --release` for honest frame times.");
    say("Type a number + Enter in this console, then move the pointer where you want it");
    say(format!("(the effect starts after {} ms):", params.delay_ms));
    say("  1 beacon     converging rings + halo, follows the pointer");
    say("  2 spotlight  dims the monitor, a clear circle closes on the pointer");
    say("  3 crosshair  lines across the whole monitor");
    say("  4 monitor    pulsing border on this monitor + a map of all monitors");
    say("  5 halo       persistent ring and glow (any command stops it)");
    say("  6 autohalo   halo whose colour follows the backdrop (dark on light, light on dark)");
    say("  7 negative   a lens showing the screen behind the pointer inverted");
    say("  8 contrast   a ring painted black or white per pixel, against what is behind");
    say("  (color=auto also makes 1-5 follow the backdrop)");
    say("  g            print the do-not-disturb signals now");
    say(format!(
        "values: size={} dips, duration={} ms, color=#{:02X}{:02X}{:02X}, dim={}, halo-seconds={}",
        params.size, params.duration_ms, params.color.r, params.color.g, params.color.b, params.dim, params.halo_seconds
    ));

    let (sender, commands) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines().map_while(std::result::Result::ok) {
            if sender.send(line.trim().to_string()).is_err() {
                break;
            }
        }
    });

    loop {
        pump();
        if let Ok(command) = commands.try_recv() {
            let style = match command.as_str() {
                "1" => "beacon",
                "2" => "spotlight",
                "3" => "crosshair",
                "4" => "monitor",
                "5" => "halo",
                "6" => "autohalo",
                "7" => "negative",
                "8" => "contrast",
                "g" => {
                    say(guard_snapshot());
                    continue;
                }
                _ => {
                    say("type 1-8 or g");
                    continue;
                }
            };
            say(format!("{style}: starting in {} ms", params.delay_ms));
            let until = Instant::now() + Duration::from_millis(params.delay_ms);
            while Instant::now() < until {
                pump();
                std::thread::sleep(Duration::from_millis(10));
            }
            say(guard_snapshot());
            play(style, &params, &commands);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn main() {
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    *START.lock().unwrap() = Some(Instant::now());
    // One log per mode, so running another probe does not overwrite it.
    let mode = std::env::args().nth(1).unwrap_or_default();
    let path = std::env::temp_dir().join(format!("finder-probe-{mode}.log"));
    *LOG.lock().unwrap() = File::create(&path).ok();
    println!("(also logging to {})", path.display());
    if mode == "shake" {
        let _ = PARAMS.set(parse_params(std::env::args().skip(2)));
    }
    if mode == "shake" {
        let q = p();
        say(format!(
            "parameters: window={}ms segment={}px speed={} reversals={} dominance={} circle={}deg cooldown={}ms ctrl-speed={} ctrl-reversals={}",
            q.window_ms, q.segment, q.speed, q.reversals, q.dominance, q.circle, q.cooldown, q.ctrl_speed, q.ctrl_reversals
        ));
    }

    match std::env::args().nth(1).as_deref() {
        Some("keys") => keys(),
        Some("shake") => shake(),
        Some("cursor") => cursor(),
        Some("guard") => guard(),
        Some("styles") => styles(),
        _ => println!("usage: finder_probe keys | shake | cursor | guard | styles"),
    }
}
