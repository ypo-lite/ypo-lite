//! A bench for the mouse finder's looks: plays the real `y_snap::finder::Player`
//! (the code the application runs) from a console, so a style can be tried,
//! tweaked and tried again without a trigger, a tray icon or a settings app.
//!
//! ```text
//! cargo run -p y-snap --example finder_lab
//! cargo run -p y-snap --example finder_lab -- --style quick
//! cargo run -p y-snap --example finder_lab -- shape=monitor all-monitors=true
//! cargo run -p y-snap --example finder_lab -- --config C:\path\to\y-snap.xml
//! ```
//!
//! It starts from the default style of `y-snap.xml` (found the way the
//! application finds it, `--config` included) or, with no file, from the
//! built-in look; `key=value` arguments are applied on top, as the `set`
//! command does. Then type commands at the prompt (`help` lists them). The
//! pointer is wherever you left it: use `in 3` to get three seconds to move it
//! to the monitor you want to look at. An empty line plays again.
//!
//! The lab plays looks only. It does not trigger on taps or shakes (try those in
//! the application), it does not move the pointer for `recall`, and it leaves
//! the file alone: `show` prints the attributes to paste into `<style>`.
//!
//! Nothing here is `unsafe` beyond what the window and the timers need; the
//! drawing, the timers and the monitor handling are the application's own.

use std::io::{BufRead, Write};
use std::sync::mpsc::{self, Receiver};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, PostMessageW, SetTimer, WM_APP, WM_TIMER,
};
use y_core::window::{self, WindowHandler, WindowSpec};
use y_snap::config::{Look, Settings, Shape, Style, finder_range as range};
use y_snap::finder::{self, Player};

/// Posted to the lab's window when the console reader has a line for it.
const WM_LINE: u32 = WM_APP + 1;
/// The timer behind `in <seconds>`.
const DELAY_TIMER: usize = 90;

const HELP: &str = "\
  (empty line), play   play the current look at the pointer
  in <seconds>         play after a delay, to put the pointer somewhere first
  stop                 end the animation now
  set k=v [k=v ...]    change the look: shape duration size color auto-color
                       dim all-monitors (a bare k=v works too)
  shapes               the shapes there are
  styles               the styles of y-snap.xml
  style <name>         take a style of y-snap.xml as the current look
  reload               read y-snap.xml again (after editing it)
  show                 print the look as <style> attributes
  quit                 leave
";

struct Lab {
    rx: Receiver<String>,
    player: Option<Player>,
    look: Look,
    name: String,
    settings: Option<Settings>,
}

fn main() {
    // The application gets this from its manifest; an example has none, and
    // without it Windows scales the surfaces and the pointer's position.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    let (tx, rx) = mpsc::channel();
    let mut lab = Lab {
        rx,
        player: None,
        look: Look::default(),
        name: "built-in".into(),
        settings: None,
    };
    lab.reload();
    lab.apply_arguments();

    let hwnd = window::create(
        &WindowSpec::hidden("YpoLite.YSnap.FinderLab", "Finder lab"),
        Box::new(lab),
    )
    .expect("the lab's window");

    // stdin blocks, so it has a thread; the lines come back as a message.
    let raw = hwnd.0 as isize;
    std::thread::spawn(move || {
        let wake = |line: String| {
            let hwnd = HWND(raw as *mut _);
            let sent = tx.send(line).is_ok();
            sent && unsafe { PostMessageW(Some(hwnd), WM_LINE, WPARAM(0), LPARAM(0)) }.is_ok()
        };
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if !wake(line) {
                return;
            }
        }
        wake("quit".into());
    });

    println!("finder lab: type `help`, or an empty line to play.");
    prompt();
    let _ = window::run_message_loop();
}

fn prompt() {
    print!("> ");
    let _ = std::io::stdout().flush();
}

impl WindowHandler for Lab {
    fn handle_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        _lparam: LPARAM,
    ) -> Option<LRESULT> {
        match msg {
            WM_LINE => {
                while let Ok(line) = self.rx.try_recv() {
                    if !self.command(hwnd, &line) {
                        window::quit(0);
                        return Some(LRESULT(0));
                    }
                }
                prompt();
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == finder::TIMER_ID => {
                if let Some(player) = self.player.as_mut() {
                    player.tick();
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == DELAY_TIMER => {
                let _ = unsafe { KillTimer(Some(hwnd), DELAY_TIMER) };
                self.play(hwnd);
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

impl Lab {
    /// Runs one line; `false` to leave.
    fn command(&mut self, hwnd: HWND, line: &str) -> bool {
        let mut words = line.split_whitespace();
        let Some(first) = words.next() else {
            self.play(hwnd);
            return true;
        };
        let rest: Vec<&str> = words.collect();
        match first {
            "play" | "p" => self.play(hwnd),
            "stop" => {
                let _ = unsafe { KillTimer(Some(hwnd), DELAY_TIMER) };
                if let Some(player) = self.player.as_mut() {
                    player.stop();
                }
            }
            "in" => match rest.first().and_then(|s| s.parse::<u32>().ok()) {
                Some(seconds) if (1..=60).contains(&seconds) => {
                    let _ = unsafe { SetTimer(Some(hwnd), DELAY_TIMER, seconds * 1000, None) };
                    println!("playing in {seconds} s");
                }
                _ => println!("in <seconds>, 1 to 60"),
            },
            "set" => self.set(&rest),
            word if word.contains('=') => {
                let all: Vec<&str> = line.split_whitespace().collect();
                self.set(&all);
            }
            "shapes" => {
                let names: Vec<_> = Shape::ALL.iter().map(|s| s.name()).collect();
                println!("{}", names.join(", "));
            }
            "styles" => self.list_styles(),
            "style" => match rest.first() {
                Some(name) => self.take_style(name),
                None => println!("style <name>; `styles` lists them"),
            },
            "reload" => {
                self.reload();
            }
            "show" => println!("{}", describe(&self.name, &self.look)),
            "help" | "?" => print!("{HELP}"),
            "quit" | "q" | "exit" => return false,
            other => println!("unknown command {other:?}; `help` lists them"),
        }
        true
    }

    fn play(&mut self, hwnd: HWND) {
        let style = Style::new(self.name.clone(), self.look);
        self.player
            .get_or_insert_with(|| Player::new(hwnd))
            .play(&style);
    }

    fn set(&mut self, pairs: &[&str]) {
        if pairs.is_empty() {
            println!("set k=v ...: shape duration size color auto-color dim all-monitors");
            return;
        }
        for pair in pairs {
            let Some((key, value)) = pair.split_once('=') else {
                println!("{pair:?}: expected key=value");
                continue;
            };
            if let Err(why) = apply(&mut self.look, key, value) {
                println!("{key}: {why}");
            }
        }
        println!("{}", describe(&self.name, &self.look));
    }

    fn list_styles(&self) {
        match &self.settings {
            Some(settings) if !settings.finder.styles.is_empty() => {
                for (index, style) in settings.finder.styles.iter().enumerate() {
                    let mark = if index == settings.finder.default_style {
                        '*'
                    } else {
                        ' '
                    };
                    println!("{mark} {}", describe(&style.name, &style.look));
                }
            }
            Some(_) => println!("y-snap.xml has no <style>"),
            None => println!("no y-snap.xml was read"),
        }
    }

    fn take_style(&mut self, name: &str) {
        let found = self
            .settings
            .as_ref()
            .and_then(|s| s.finder.styles.iter().find(|style| style.name == name));
        match found {
            Some(style) => {
                self.look = style.look;
                self.name = style.name.clone();
                println!("{}", describe(&self.name, &self.look));
            }
            None => println!("no style named {name:?}; `styles` lists them"),
        }
    }

    /// Reads `y-snap.xml` and takes its default style as the current look.
    fn reload(&mut self) {
        match Settings::load() {
            Ok(settings) => {
                for warning in &settings.warnings {
                    println!("warning: {warning}");
                }
                let style = settings.finder.style(None);
                self.look = style.look;
                self.name = style.name;
                self.settings = Some(settings);
                println!("y-snap.xml read; {}", describe(&self.name, &self.look));
            }
            Err(error) => {
                self.settings = None;
                println!("{error:?}; using the current look");
            }
        }
    }

    /// `--style name`, `key=value`: what the command line says, on top of the
    /// file. (`--config <path>` is read by `Settings::load` itself.)
    fn apply_arguments(&mut self) {
        let mut arguments = std::env::args().skip(1);
        let mut pairs = Vec::new();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--config" | "-config" => {
                    arguments.next();
                }
                "--style" => match arguments.next() {
                    Some(name) => self.take_style(&name),
                    None => println!("--style needs a name"),
                },
                _ if argument.contains('=') => pairs.push(argument),
                _ => println!("ignored argument {argument:?}"),
            }
        }
        if !pairs.is_empty() {
            let pairs: Vec<&str> = pairs.iter().map(String::as_str).collect();
            self.set(&pairs);
        }
    }
}

/// One attribute of a look, checked against the ranges the file obeys.
fn apply(look: &mut Look, key: &str, value: &str) -> Result<(), String> {
    let number = |range: std::ops::RangeInclusive<u32>| -> Result<u32, String> {
        let n: u32 = value
            .parse()
            .map_err(|_| format!("{value:?} is not a number"))?;
        if range.contains(&n) {
            Ok(n)
        } else {
            Err(format!("{n} is outside {}..{}", range.start(), range.end()))
        }
    };
    let flag = || match value {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(format!("{value:?} is not true or false")),
    };
    match key {
        "shape" => {
            look.shape = Shape::from_name(value).ok_or("unknown shape; `shapes` lists them")?;
        }
        "duration" => look.duration_ms = number(range::DURATION_MS)?,
        "size" => look.size = number(range::SIZE)?,
        "dim" => look.dim = number(range::DIM)?,
        "color" => {
            look.color = y_core::overlay::Color::parse(value).ok_or("not a colour (#RRGGBB)")?
        }
        "auto-color" => look.auto_color = flag()?,
        "all-monitors" => look.all_monitors = flag()?,
        "recall" => return Err("the lab never moves the pointer".into()),
        _ => return Err("unknown attribute".into()),
    }
    Ok(())
}

/// The look as the attributes of a `<style>`, ready to paste.
fn describe(name: &str, look: &Look) -> String {
    format!(
        "<style name=\"{name}\" shape=\"{}\" duration=\"{}\" size=\"{}\" color=\"#{:02X}{:02X}{:02X}\" auto-color=\"{}\" dim=\"{}\" all-monitors=\"{}\"/>",
        look.shape,
        look.duration_ms,
        look.size,
        look.color.r,
        look.color.g,
        look.color.b,
        look.auto_color,
        look.dim,
        look.all_monitors,
    )
}
