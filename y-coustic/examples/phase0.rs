//! Phase 0 probe for y-coustic. Throwaway: delete it once the answers are in
//! `docs/design/y-coustic-plan.md`.
//!
//! Run it from a terminal, it logs there and reads commands from there:
//!
//! ```sh
//! cargo run -p y-coustic --example phase0
//! ```
//!
//! While it runs, the three volume keys are swallowed and applied through
//! `IAudioEndpointVolume` (the `pass` command gives them back to Windows).
//! Quitting with `q`, or killing the process, restores everything except any
//! level changed with `set`, whose previous value is printed.
//!
//! It adds two tray icons, which Windows 11 may put in the overflow area:
//! `A` has `NIF_SHOWTIP` as `y_core::tray` does, `B` does not.

use std::io::BufRead;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, S_OK, WPARAM};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, EDataFlow, ERole, IAudioSessionControl2, IAudioSessionManager2, IMMDevice,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eCapture, eCommunications,
    eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_K, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION,
    NIN_POPUPCLOSE, NIN_POPUPOPEN, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
    Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HC_ACTION,
    HSHELL_APPCOMMAND, IDI_APPLICATION, IDI_INFORMATION, KBDLLHOOKSTRUCT, LoadIconW, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterShellHookWindow, RegisterWindowMessageW,
    SetWindowsHookExW, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WM_APP, WM_KEYDOWN, WM_MOUSEMOVE,
    WM_SYSKEYDOWN, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{Interface, PWSTR, Result, w};

const WM_KEY_EVENT: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
const WM_COMMAND_LINE: u32 = WM_APP + 3;

const ICON_WITH_TIP: u32 = 1;
const ICON_WITHOUT_TIP: u32 = 2;
const STEP: f32 = 0.05;

static OWNER: AtomicIsize = AtomicIsize::new(0);
static SWALLOW: AtomicBool = AtomicBool::new(true);
static SHELL_MESSAGE: AtomicU32 = AtomicU32::new(0);
static START: OnceLock<Instant> = OnceLock::new();

/// Writes a line with a millisecond stamp, for reading delays off the log.
macro_rules! log {
    ($($arg:tt)*) => {
        println!(
            "[{:>8} ms] {}",
            START.get_or_init(Instant::now).elapsed().as_millis(),
            format_args!($($arg)*),
        )
    };
}

/// A session from the last `sessions` listing, the target of `set`.
struct Session {
    label: String,
    volume: ISimpleAudioVolume,
}

thread_local! {
    static SESSIONS: std::cell::RefCell<Vec<Session>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn main() -> Result<()> {
    START.get_or_init(Instant::now);
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()?;

    let instance = unsafe { GetModuleHandleW(None)? };
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        lpszClassName: w!("y-coustic-phase0"),
        ..Default::default()
    };
    unsafe { RegisterClassW(&class) };

    // A hidden top-level window: `RegisterShellHookWindow` wants one, and it
    // is never shown.
    let owner = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("y-coustic-phase0"),
            w!("y-coustic phase 0"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?
    };
    OWNER.store(owner.0 as isize, Ordering::SeqCst);

    let shell_message = unsafe { RegisterWindowMessageW(w!("SHELLHOOK")) };
    SHELL_MESSAGE.store(shell_message, Ordering::SeqCst);
    let shell_hook = unsafe { RegisterShellHookWindow(owner) }.as_bool();
    log!("shell hook window registered: {shell_hook}");

    let _hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook),
            Some(instance.into()),
            0,
        )?
    };
    log!("keyboard hook installed, volume keys are swallowed");

    add_icon(owner, ICON_WITH_TIP, true, "phase0 A: NIF_SHOWTIP")?;
    add_icon(owner, ICON_WITHOUT_TIP, false, "phase0 B: no NIF_SHOWTIP")?;
    log!("tray icons added: A with NIF_SHOWTIP, B without");

    print_help();
    spawn_stdin_reader();

    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
        unsafe { DispatchMessageW(&message) };
    }

    remove_icon(owner, ICON_WITH_TIP);
    remove_icon(owner, ICON_WITHOUT_TIP);
    Ok(())
}

fn print_help() {
    println!(
        "\nCommands:\n\
         \x20 swallow | pass      volume keys handled here, or left to Windows\n\
         \x20 up | down | mute    change the master through the API, with no key\n\
         \x20 sessions            list render and capture sessions, numbered\n\
         \x20 set <n> <percent>   SetMasterVolume on session n, then read it back\n\
         \x20 mic                 level and mute of the default capture endpoints\n\
         \x20 micmute             toggle the mute of the default capture endpoint\n\
         \x20 help | q\n"
    );
}

/// Commands arrive on the window's thread, where the COM objects live.
fn spawn_stdin_reader() {
    std::thread::spawn(|| {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let boxed = Box::into_raw(Box::new(line));
            let posted = unsafe {
                PostMessageW(
                    Some(owner()),
                    WM_COMMAND_LINE,
                    WPARAM(0),
                    LPARAM(boxed as isize),
                )
            };
            if posted.is_err() {
                break;
            }
        }
    });
}

fn owner() -> HWND {
    HWND(OWNER.load(Ordering::SeqCst) as *mut _)
}

fn held(key: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(key.0 as i32) as u16 & 0x8000 != 0 }
}

/// Keys a dedicated laptop button might send, logged and never swallowed.
/// Deliberately not every key: what is typed stays out of the log. Covers
/// media and launch keys, F13 to F24, the "no key" codes, and `Win+Alt+K`,
/// the Windows 11 shortcut to mute a call.
fn is_special_key(key: VIRTUAL_KEY) -> bool {
    matches!(key.0, 0 | 0x7C..=0x87 | 0xA6..=0xB7 | 0xFF)
        || (key == VK_K && held(VK_MENU) && (held(VK_LWIN) || held(VK_RWIN)))
}

/// No COM and no logging here: the event is posted to the window.
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let key = VIRTUAL_KEY(info.vkCode as u16);
        let volume = matches!(key, VK_VOLUME_UP | VK_VOLUME_DOWN | VK_VOLUME_MUTE);
        if volume || is_special_key(key) {
            let message = wparam.0 as u32;
            let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
            let swallow = volume && SWALLOW.load(Ordering::SeqCst);

            let mut packed = key.0 as usize;
            packed |= (down as usize) << 16;
            packed |= (swallow as usize) << 17;
            packed |= (held(VK_CONTROL) as usize) << 18;
            packed |= (held(VK_SHIFT) as usize) << 19;
            packed |= (held(VK_MENU) as usize) << 20;
            packed |= ((held(VK_LWIN) || held(VK_RWIN)) as usize) << 21;
            let detail = ((info.scanCode & 0xFFFF) | (info.flags.0 << 16)) as isize;

            let _ = unsafe {
                PostMessageW(Some(owner()), WM_KEY_EVENT, WPARAM(packed), LPARAM(detail))
            };
            if swallow {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_KEY_EVENT => on_key(wparam.0, lparam.0),
        WM_TRAY => on_tray(window, wparam.0, lparam.0 as u32),
        WM_COMMAND_LINE => {
            let line = unsafe { Box::from_raw(lparam.0 as *mut String) };
            on_command(&line);
        }
        _ if message == SHELL_MESSAGE.load(Ordering::SeqCst) => on_shell(wparam.0, lparam.0),
        _ => return unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
    LRESULT(0)
}

fn on_key(packed: usize, detail: isize) {
    let key = VIRTUAL_KEY((packed & 0xFFFF) as u16);
    let down = packed & (1 << 16) != 0;
    let swallowed = packed & (1 << 17) != 0;

    let name = match key {
        VK_VOLUME_UP => "VolumeUp".to_owned(),
        VK_VOLUME_DOWN => "VolumeDown".to_owned(),
        VK_VOLUME_MUTE => "VolumeMute".to_owned(),
        other => format!("vk {:#x}", other.0),
    };
    let mut held = String::new();
    for (bit, modifier) in [(18, "Ctrl+"), (19, "Shift+"), (20, "Alt+"), (21, "Win+")] {
        if packed & (1 << bit) != 0 {
            held.push_str(modifier);
        }
    }
    log!(
        "key {held}{name} {} {} scan={:#x} flags={:#x}",
        if down { "down" } else { "up" },
        if swallowed { "swallowed" } else { "passed" },
        detail & 0xFFFF,
        (detail >> 16) & 0xFFFF,
    );

    if down && swallowed {
        let change = match key {
            VK_VOLUME_UP => step_master(STEP),
            VK_VOLUME_DOWN => step_master(-STEP),
            _ => toggle_master_mute(),
        };
        report("master", change);
    }
}

/// `WM_APPCOMMAND` as the shell sees it. This only observes: it cannot stop
/// the command. It shows what a device that bypasses the keyboard hook sends.
fn on_shell(wparam: usize, lparam: isize) {
    if (wparam & 0x7FFF) as u32 != HSHELL_APPCOMMAND {
        return;
    }
    let command = ((lparam as usize >> 16) & 0x0FFF) as u32;
    let name = match command {
        8 => "APPCOMMAND_VOLUME_MUTE",
        9 => "APPCOMMAND_VOLUME_DOWN",
        10 => "APPCOMMAND_VOLUME_UP",
        24 => "APPCOMMAND_MICROPHONE_VOLUME_MUTE",
        25 => "APPCOMMAND_MICROPHONE_VOLUME_DOWN",
        26 => "APPCOMMAND_MICROPHONE_VOLUME_UP",
        44 => "APPCOMMAND_MIC_ON_OFF_TOGGLE",
        _ => "other",
    };
    log!("shell appcommand {command} {name}");
}

fn on_tray(window: HWND, wparam: usize, lparam: u32) {
    let event = lparam & 0xFFFF;
    let icon = (lparam >> 16) & 0xFFFF;
    let letter = if icon == ICON_WITH_TIP { 'A' } else { 'B' };
    let x = (wparam & 0xFFFF) as i16;
    let y = ((wparam >> 16) & 0xFFFF) as i16;

    match event {
        WM_MOUSEMOVE => log!(
            "tray {letter} WM_MOUSEMOVE at {x},{y} rect={}",
            describe_rect(icon_rect(window, icon))
        ),
        NIN_POPUPOPEN => log!("tray {letter} NIN_POPUPOPEN at {x},{y}"),
        NIN_POPUPCLOSE => log!("tray {letter} NIN_POPUPCLOSE"),
        other => log!("tray {letter} event {other:#x}"),
    }
}

fn icon_rect(window: HWND, id: u32) -> Option<RECT> {
    let identifier = NOTIFYICONIDENTIFIER {
        cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: window,
        uID: id,
        ..Default::default()
    };
    unsafe { Shell_NotifyIconGetRect(&identifier) }.ok()
}

fn describe_rect(rect: Option<RECT>) -> String {
    rect.map_or("none".into(), |r| {
        format!("{},{}..{},{}", r.left, r.top, r.right, r.bottom)
    })
}

fn add_icon(window: HWND, id: u32, show_tip: bool, tip: &str) -> Result<()> {
    let icon = if show_tip {
        IDI_APPLICATION
    } else {
        IDI_INFORMATION
    };
    let mut flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    if show_tip {
        flags |= NIF_SHOWTIP;
    }

    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: id,
        uFlags: flags,
        uCallbackMessage: WM_TRAY,
        hIcon: unsafe { LoadIconW(None, icon)? },
        ..Default::default()
    };
    for (slot, unit) in data.szTip.iter_mut().zip(tip.encode_utf16()) {
        *slot = unit;
    }

    unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.ok()?;
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) }.ok()
}

fn remove_icon(window: HWND, id: u32) {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: id,
        ..Default::default()
    };
    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
}

fn on_command(line: &str) {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        [] => {}
        ["q"] | ["quit"] => unsafe { PostQuitMessage(0) },
        ["help"] => print_help(),
        ["swallow"] => {
            SWALLOW.store(true, Ordering::SeqCst);
            log!("volume keys are swallowed");
        }
        ["pass"] => {
            SWALLOW.store(false, Ordering::SeqCst);
            log!("volume keys go to Windows");
        }
        ["up"] => report("master, no key", step_master(STEP)),
        ["down"] => report("master, no key", step_master(-STEP)),
        ["mute"] => report("master, no key", toggle_master_mute()),
        ["mic"] => {
            report("mic console", mic_state(eConsole));
            report("mic communications", mic_state(eCommunications));
        }
        ["micmute"] => report("mic console", toggle_mic_mute()),
        ["sessions"] => {
            if let Err(error) = list_sessions() {
                log!("listing failed: {error}");
            }
        }
        ["set", index, percent] => match (index.parse(), percent.parse()) {
            (Ok(index), Ok(percent)) => set_session(index, percent),
            _ => log!("usage: set <n> <percent>"),
        },
        _ => log!("unknown command, try help"),
    }
}

fn report(what: &str, result: Result<String>) {
    match result {
        Ok(state) => log!("{what}: {state}"),
        Err(error) => log!("{what}: failed: {error}"),
    }
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

fn master() -> Result<IAudioEndpointVolume> {
    let device = unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole)? };
    unsafe { device.Activate(CLSCTX_ALL, None) }
}

fn step_master(change: f32) -> Result<String> {
    let endpoint = master()?;
    let level = unsafe { endpoint.GetMasterVolumeLevelScalar()? };
    let target = (level + change).clamp(0.0, 1.0);
    unsafe { endpoint.SetMasterVolumeLevelScalar(target, std::ptr::null())? };
    let read = unsafe { endpoint.GetMasterVolumeLevelScalar()? };
    Ok(format!("{:.0} % -> {:.0} %", level * 100.0, read * 100.0))
}

fn toggle_master_mute() -> Result<String> {
    let endpoint = master()?;
    let muted = unsafe { endpoint.GetMute()? }.as_bool();
    unsafe { endpoint.SetMute(!muted, std::ptr::null())? };
    let read = unsafe { endpoint.GetMute()? }.as_bool();
    Ok(format!("mute {muted} -> {read}"))
}

fn mic(role: ERole) -> Result<(String, IAudioEndpointVolume)> {
    let device = unsafe { enumerator()?.GetDefaultAudioEndpoint(eCapture, role)? };
    let id = device_id(&device)?;
    Ok((id, unsafe { device.Activate(CLSCTX_ALL, None)? }))
}

fn mic_state(role: ERole) -> Result<String> {
    let (id, endpoint) = mic(role)?;
    let level = unsafe { endpoint.GetMasterVolumeLevelScalar()? };
    let muted = unsafe { endpoint.GetMute()? }.as_bool();
    Ok(format!("{:.0} %, muted={muted}, {id}", level * 100.0))
}

fn toggle_mic_mute() -> Result<String> {
    let (_, endpoint) = mic(eConsole)?;
    let muted = unsafe { endpoint.GetMute()? }.as_bool();
    unsafe { endpoint.SetMute(!muted, std::ptr::null())? };
    let read = unsafe { endpoint.GetMute()? }.as_bool();
    Ok(format!("mute {muted} -> {read}"))
}

/// Takes and frees a string the callee allocated.
fn take_string(pointer: PWSTR) -> String {
    let text = unsafe { pointer.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(pointer.0 as *const _)) };
    text
}

fn device_id(device: &IMMDevice) -> Result<String> {
    Ok(take_string(unsafe { device.GetId()? }))
}

fn list_sessions() -> Result<()> {
    let mut found = Vec::new();
    list_flow(eRender, "render", &mut found)?;
    list_flow(eCapture, "capture", &mut found)?;
    SESSIONS.with_borrow_mut(|sessions| *sessions = found);
    Ok(())
}

fn list_flow(flow: EDataFlow, name: &str, found: &mut Vec<Session>) -> Result<()> {
    let enumerator = enumerator()?;
    let default = unsafe { enumerator.GetDefaultAudioEndpoint(flow, eConsole) }
        .and_then(|device| device_id(&device))
        .unwrap_or_default();

    let devices = unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)? };
    for index in 0..unsafe { devices.GetCount()? } {
        let device = unsafe { devices.Item(index)? };
        let id = device_id(&device)?;
        let mark = if id == default { " (default)" } else { "" };
        println!("{name} device {index}{mark}: {id}");

        let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let sessions = unsafe { manager.GetSessionEnumerator()? };
        for position in 0..unsafe { sessions.GetCount()? } {
            let control = unsafe { sessions.GetSession(position)? };
            let details = control.cast::<IAudioSessionControl2>()?;
            let volume = control.cast::<ISimpleAudioVolume>()?;

            let pid = unsafe { details.GetProcessId() }.unwrap_or(0);
            let system = unsafe { details.IsSystemSoundsSession() } == S_OK;
            let state = match unsafe { control.GetState() }.map(|state| state.0) {
                Ok(0) => "inactive",
                Ok(1) => "active",
                Ok(2) => "expired",
                _ => "unknown",
            };
            let exe = if system {
                "(system sounds)".to_owned()
            } else {
                y_core::process::executable_stem(pid).unwrap_or_else(|| "?".to_owned())
            };
            let level = unsafe { volume.GetMasterVolume()? };
            let muted = unsafe { volume.GetMute()? }.as_bool();

            let label = format!("{name} {exe} pid {pid}");
            println!(
                "  [{}] {label}: {state}, {:.0} %, muted={muted}",
                found.len(),
                level * 100.0
            );
            found.push(Session { label, volume });
        }
    }
    Ok(())
}

fn set_session(index: usize, percent: f32) {
    SESSIONS.with_borrow(|sessions| {
        let Some(session) = sessions.get(index) else {
            log!("no session {index}, run sessions first");
            return;
        };
        let target = (percent / 100.0).clamp(0.0, 1.0);
        let result = (|| unsafe {
            let before = session.volume.GetMasterVolume()?;
            session.volume.SetMasterVolume(target, std::ptr::null())?;
            let after = session.volume.GetMasterVolume()?;
            Ok(format!(
                "{:.0} % -> {:.0} % (restore with: set {index} {:.0})",
                before * 100.0,
                after * 100.0,
                before * 100.0
            ))
        })();
        report(&session.label, result);
    });
}
