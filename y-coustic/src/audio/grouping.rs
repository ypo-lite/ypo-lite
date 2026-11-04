//! From audio sessions to applications, and from a window to its application.
//! Pure: no COM, no Win32, every rule tested with fixtures.
//!
//! Core Audio knows *sessions*: one per process (more, on several devices) that
//! opened a stream. A user knows *applications*, and a window's process is often
//! not the one playing: Chrome and Discord hold their session in a helper
//! process, a Store app's window belongs to `ApplicationFrameHost`. So:
//!
//! * sessions are grouped by **executable path**, which folds a browser's audio
//!   helper and its main process into one row, and the same application playing
//!   on two output devices into one row too;
//! * system sounds are one row, a session of no single process (pid 0, not
//!   system sounds) is a row of its own;
//! * a window finds its application by **the same executable**, then by a
//!   session **descended from the window's process**.
//!
//! An application's level is its loudest session's, and it is muted only when
//! every session is.

use std::collections::HashMap;

use y_coustic::model::{Action, Change, Kind, Level, State};

/// What Core Audio says about one session, with nothing of COM left in it.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub pid: u32,
    /// The full path of the owning process's executable, `None` when it could
    /// not be read (an elevated process) or there is no single process.
    pub exe: Option<String>,
    pub is_system: bool,
    /// The name the session gave itself; only for a row with no executable.
    pub display_name: String,
    pub level: Level,
    pub muted: bool,
    /// Windows is about to forget it: never shown, never written.
    pub expired: bool,
}

/// What makes sessions one application.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AppKey {
    /// The same executable, compared without regard to case.
    Exe(String),
    /// An owner whose executable is unknown: the process alone.
    Pid(u32),
    /// The system sounds of every device.
    System,
    /// A session of no single process, one row each (its index in the list).
    Session(usize),
}

/// One row: the sessions that make up an application.
#[derive(Debug, Clone, PartialEq)]
pub struct Application {
    pub key: AppKey,
    /// Indices into the session list this was grouped from.
    pub members: Vec<usize>,
    /// The first member's executable path, as written, for the icon and name.
    pub exe: Option<String>,
    /// The first member's own name: what a row with no executable is called.
    pub display_name: String,
    /// The loudest member.
    pub level: Level,
    /// Every member is muted.
    pub muted: bool,
}

impl Application {
    pub fn is_system(&self) -> bool {
        self.key == AppKey::System
    }
}

/// The applications of `sessions`, in order of first appearance. Expired
/// sessions are left out.
pub fn group(sessions: &[SessionInfo]) -> Vec<Application> {
    let mut applications: Vec<Application> = Vec::new();
    let mut index_of: HashMap<AppKey, usize> = HashMap::new();

    for (position, session) in sessions.iter().enumerate() {
        if session.expired {
            continue;
        }
        let key = key_of(position, session);
        let slot = *index_of.entry(key.clone()).or_insert_with(|| {
            applications.push(Application {
                key,
                members: Vec::new(),
                exe: session.exe.clone(),
                display_name: session.display_name.clone(),
                level: 0.0,
                muted: true,
            });
            applications.len() - 1
        });
        applications[slot].members.push(position);
    }

    for application in &mut applications {
        application.level = application
            .members
            .iter()
            .map(|&member| sessions[member].level)
            .fold(0.0, f32::max);
        application.muted = application
            .members
            .iter()
            .all(|&member| sessions[member].muted);
    }
    applications
}

fn key_of(position: usize, session: &SessionInfo) -> AppKey {
    if session.is_system {
        AppKey::System
    } else if let Some(exe) = &session.exe {
        AppKey::Exe(exe.to_lowercase())
    } else if session.pid != 0 {
        AppKey::Pid(session.pid)
    } else {
        AppKey::Session(position)
    }
}

/// The process a window stands for, as far as the window can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowProcess {
    pub pid: u32,
    pub exe: Option<String>,
}

/// How far up the process tree a descendant check climbs: a guard against a
/// loop made by a reused pid, not a limit anyone reaches.
const MAX_DEPTH: usize = 32;

/// Whether `pid` is `ancestor` or has it somewhere above in `parents`.
fn descends_from(pid: u32, ancestor: u32, parents: &HashMap<u32, u32>) -> bool {
    let mut current = pid;
    for _ in 0..MAX_DEPTH {
        if current == ancestor {
            return true;
        }
        match parents.get(&current) {
            Some(&parent) if parent != current && parent != 0 => current = parent,
            _ => return false,
        }
    }
    false
}

/// The application a window belongs to: the one with the window's executable,
/// else the first with a session descended from the window's process. System
/// sounds never answer: no window is the system sounds.
pub fn for_window(
    applications: &[Application],
    sessions: &[SessionInfo],
    window: &WindowProcess,
    parents: &HashMap<u32, u32>,
) -> Option<usize> {
    if let Some(exe) = &window.exe {
        let wanted = AppKey::Exe(exe.to_lowercase());
        let found = applications
            .iter()
            .position(|application| application.key == wanted);
        if found.is_some() {
            return found;
        }
    }
    if window.pid == 0 {
        return None;
    }
    applications.iter().position(|application| {
        !application.is_system()
            && application.members.iter().any(|&member| {
                let pid = sessions[member].pid;
                pid != 0 && descends_from(pid, window.pid, parents)
            })
    })
}

/// What to write to each session of `application` for `change`, as
/// `(session index, new state)`. Each session steps on its own, so levels
/// keep their relation; a mute is one decision for the whole application
/// (mute all if any is audible, else unmute all), so sessions cannot diverge.
pub fn apply(
    application: &Application,
    sessions: &[SessionInfo],
    change: Change,
    step_pct: f32,
) -> Vec<(usize, State)> {
    let mute_all = application
        .members
        .iter()
        .any(|&member| !sessions[member].muted);

    application
        .members
        .iter()
        .map(|&member| {
            let before = State {
                level: sessions[member].level,
                muted: sessions[member].muted,
            };
            let after = match change {
                Change::Action(Action::Mute) => State {
                    muted: mute_all,
                    ..before
                },
                other => other.apply(Kind::App, before, step_pct),
            };
            (member, after)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(pid: u32, exe: Option<&str>, level: f32, muted: bool) -> SessionInfo {
        SessionInfo {
            pid,
            exe: exe.map(str::to_owned),
            is_system: false,
            display_name: String::new(),
            level,
            muted,
            expired: false,
        }
    }

    fn system(level: f32) -> SessionInfo {
        SessionInfo {
            is_system: true,
            ..session(0, None, level, false)
        }
    }

    const CHROME: &str = r"C:\Program Files\Google\Chrome\chrome.exe";

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn window(pid: u32, exe: Option<&str>) -> WindowProcess {
        WindowProcess {
            pid,
            exe: exe.map(str::to_owned),
        }
    }

    #[test]
    fn sessions_of_one_executable_are_one_application() {
        // Chrome's audio helper and its main process.
        let sessions = [
            session(100, Some(CHROME), 0.4, false),
            session(250, Some(&CHROME.to_uppercase()), 0.9, true),
            session(300, Some(r"C:\Tools\player.exe"), 0.5, false),
        ];
        let applications = group(&sessions);
        assert_eq!(applications.len(), 2);
        assert_eq!(applications[0].members, [0, 1]);
        assert!(near(applications[0].level, 0.9), "the loudest member");
        assert!(!applications[0].muted, "muted only if all are");
        assert_eq!(applications[1].members, [2]);
    }

    #[test]
    fn muted_means_every_session_muted() {
        let sessions = [
            session(1, Some(CHROME), 0.4, true),
            session(2, Some(CHROME), 0.4, true),
        ];
        assert!(group(&sessions)[0].muted);
    }

    #[test]
    fn system_sounds_are_one_row_across_devices() {
        let sessions = [
            system(0.3),
            session(5, Some(CHROME), 0.5, false),
            system(0.6),
        ];
        let applications = group(&sessions);
        assert_eq!(applications.len(), 2);
        assert!(applications[0].is_system());
        assert_eq!(applications[0].members, [0, 2]);
    }

    #[test]
    fn a_session_of_no_single_process_is_its_own_row() {
        let sessions = [session(0, None, 0.5, false), session(0, None, 0.5, false)];
        assert_eq!(group(&sessions).len(), 2);
    }

    #[test]
    fn an_unreadable_executable_groups_by_process() {
        let sessions = [
            session(7, None, 0.5, false),
            session(7, None, 0.2, false),
            session(8, None, 0.2, false),
        ];
        let applications = group(&sessions);
        assert_eq!(applications.len(), 2);
        assert_eq!(applications[0].members, [0, 1]);
    }

    #[test]
    fn expired_sessions_are_left_out() {
        let mut gone = session(1, Some(CHROME), 0.5, false);
        gone.expired = true;
        let sessions = [gone, session(2, Some(r"C:\a.exe"), 0.5, false)];
        let applications = group(&sessions);
        assert_eq!(applications.len(), 1);
        assert_eq!(applications[0].members, [1]);
    }

    #[test]
    fn a_window_finds_the_application_with_its_executable() {
        // The window belongs to pid 100; the audio runs in helper 250.
        let sessions = [
            session(300, Some(r"C:\Tools\player.exe"), 0.5, false),
            session(250, Some(CHROME), 0.9, false),
        ];
        let applications = group(&sessions);
        let found = for_window(
            &applications,
            &sessions,
            &window(100, Some(CHROME)),
            &HashMap::new(),
        );
        assert_eq!(found, Some(1));
    }

    #[test]
    fn a_window_finds_a_helper_with_another_executable_by_descent() {
        // Audio in `audio-host.exe`, started by the window's own process.
        let sessions = [session(900, Some(r"C:\App\audio-host.exe"), 0.5, false)];
        let applications = group(&sessions);
        let parents = HashMap::from([(900, 800), (800, 700), (700, 4)]);
        let app = window(700, Some(r"C:\App\app.exe"));
        assert_eq!(
            for_window(&applications, &sessions, &app, &parents),
            Some(0)
        );
        // A window elsewhere in the tree does not.
        let other = window(701, Some(r"C:\Other\other.exe"));
        assert_eq!(for_window(&applications, &sessions, &other, &parents), None);
    }

    #[test]
    fn a_window_with_no_audio_finds_nothing() {
        let sessions = [session(1, Some(CHROME), 0.5, false)];
        let applications = group(&sessions);
        let notepad = window(50, Some(r"C:\Windows\notepad.exe"));
        assert_eq!(
            for_window(&applications, &sessions, &notepad, &HashMap::new()),
            None
        );
    }

    #[test]
    fn the_system_sounds_are_nobodys_window() {
        let sessions = [system(0.5)];
        let applications = group(&sessions);
        // pid 0 is also what a failed lookup gives; neither matches.
        let nobody = window(0, None);
        assert_eq!(
            for_window(&applications, &sessions, &nobody, &HashMap::new()),
            None
        );
        let child = window(4, None);
        let parents = HashMap::from([(0, 0)]);
        assert_eq!(for_window(&applications, &sessions, &child, &parents), None);
    }

    #[test]
    fn a_loop_in_the_parent_table_ends() {
        let sessions = [session(10, Some(r"C:\a.exe"), 0.5, false)];
        let applications = group(&sessions);
        let parents = HashMap::from([(10, 20), (20, 10)]);
        assert_eq!(
            for_window(&applications, &sessions, &window(99, None), &parents),
            None
        );
    }

    #[test]
    fn a_step_moves_each_session_and_keeps_their_relation() {
        let sessions = [
            session(1, Some(CHROME), 0.4, false),
            session(2, Some(CHROME), 0.8, false),
        ];
        let applications = group(&sessions);
        let down = Change::Action(Action::VolumeDown);
        let written = apply(&applications[0], &sessions, down, 5.0);
        assert_eq!(written.len(), 2);
        assert!(near(written[0].1.level, 0.35));
        assert!(near(written[1].1.level, 0.75));
    }

    #[test]
    fn a_step_up_unmutes_the_application() {
        let sessions = [session(1, Some(CHROME), 0.4, true)];
        let applications = group(&sessions);
        let written = apply(&applications[0], &sessions, Change::Scroll(1.0), 5.0);
        assert!(!written[0].1.muted);
        assert!(near(written[0].1.level, 0.45));
    }

    #[test]
    fn mute_is_one_decision_for_all_the_sessions() {
        let mute = Change::Action(Action::Mute);
        let sessions = [
            session(1, Some(CHROME), 0.4, true),
            session(2, Some(CHROME), 0.8, false),
        ];
        let applications = group(&sessions);
        // One is audible: mute them all, so they cannot diverge.
        let muted = apply(&applications[0], &sessions, mute, 5.0);
        assert!(muted.iter().all(|(_, state)| state.muted));
        assert!(near(muted[1].1.level, 0.8), "levels are untouched");

        let all_muted = [
            session(1, Some(CHROME), 0.4, true),
            session(2, Some(CHROME), 0.8, true),
        ];
        let applications = group(&all_muted);
        let unmuted = apply(&applications[0], &all_muted, mute, 5.0);
        assert!(unmuted.iter().all(|(_, state)| !state.muted));
    }

    #[test]
    fn set_volume_sets_every_session() {
        let sessions = [
            session(1, Some(CHROME), 0.4, false),
            session(2, Some(CHROME), 0.8, false),
        ];
        let applications = group(&sessions);
        let set = Change::Action(Action::SetVolume(30));
        let written = apply(&applications[0], &sessions, set, 5.0);
        assert!(written.iter().all(|(_, state)| near(state.level, 0.30)));
    }

    #[test]
    fn one_program_on_two_output_devices_is_one_row() {
        // The design's two-endpoint Terminal.
        let terminal = r"C:\Program Files\WindowsApps\Terminal\wt.exe";
        let sessions = [
            session(40, Some(terminal), 0.5, false),
            session(40, Some(terminal), 0.2, false),
        ];
        let applications = group(&sessions);
        assert_eq!(applications.len(), 1);
        assert!(near(applications[0].level, 0.5));
    }
}
