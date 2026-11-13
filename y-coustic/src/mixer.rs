//! What a change does to the audio, and what the audio looks like now: the
//! bridge between the pure rules (`model`, `audio::grouping`) and COM
//! (`audio`), shared by the bindings and the panel.
//!
//! Nothing is held between calls. Every function resolves its devices and
//! sessions afresh and drops them: a changed default device, an application
//! that began to play or stopped needs no notification code, and between
//! actions the program keeps no audio object.

use windows::core::Result;

use y_coustic::model::{Change, Kind, State, Target};

use crate::audio::grouping::{self, AppKey, Application, WindowProcess};
use crate::audio::sessions::Sessions;
use crate::audio::{Endpoint, Flow, Microphones, Role};
use y_coustic::look::AppRef;

/// Which line of the mixer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowId {
    Master,
    Mic,
    App(AppKey),
}

/// One line of the mixer, as it reads now.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: RowId,
    pub kind: Kind,
    /// Who an application row is; `None` for the master and the microphone.
    pub app: Option<AppRef>,
    /// `None` when there is no such device (no microphone, no output).
    pub state: Option<State>,
}

/// The master, the microphone, then the applications that have a session, the
/// system sounds last (and only with `system_sounds`).
pub fn read_rows(system_sounds: bool) -> Vec<Row> {
    let mut rows = vec![
        Row {
            id: RowId::Master,
            kind: Kind::Master,
            app: None,
            state: master_state().ok(),
        },
        Row {
            id: RowId::Mic,
            kind: Kind::Mic,
            app: None,
            state: Microphones::resolve()
                .and_then(|microphones| microphones.state())
                .ok(),
        },
    ];

    if let Ok(sessions) = Sessions::snapshot() {
        let applications = grouping::group(sessions.infos());
        let (system, others): (Vec<_>, Vec<_>) = applications
            .iter()
            .partition(|application| application.is_system());
        let shown = others
            .into_iter()
            .chain(system.into_iter().filter(|_| system_sounds));
        rows.extend(shown.map(|application| Row {
            id: RowId::App(application.key.clone()),
            kind: Kind::App,
            app: Some(app_ref(application)),
            state: Some(State {
                level: application.level,
                muted: application.muted,
            }),
        }));
    }
    rows
}

/// Applies `change` to one row. Fails when the device or the application is
/// gone.
pub fn apply_row(id: &RowId, change: Change, step_pct: f32) -> Result<State> {
    match id {
        RowId::Master => apply_to_master(change, step_pct),
        RowId::Mic => apply_to_microphone(change, step_pct),
        RowId::App(key) => {
            let sessions = Sessions::snapshot()?;
            let applications = grouping::group(sessions.infos());
            let application = applications
                .iter()
                .find(|application| application.key == *key)
                .ok_or_else(windows::core::Error::empty)?;
            Ok(write_application(&sessions, application, change, step_pct))
        }
    }
}

/// The row of the application behind the focused or the pointed window, if it
/// has a session: what the panel selects when it opens. Read before the panel
/// takes the focus, which changes the answer.
pub fn row_of_window(target: Target) -> Option<RowId> {
    let (_, applications, found) = locate(target)?;
    Some(RowId::App(applications[found].key.clone()))
}

fn master_state() -> Result<State> {
    let endpoint = Endpoint::default(Flow::Render, Role::Console)?;
    Ok(State {
        level: endpoint.level()?,
        muted: endpoint.muted()?,
    })
}

/// Applies `change` to the default output: the device is resolved now, so a
/// changed default is followed with no reload. Returns the state written.
pub fn apply_to_master(change: Change, step_pct: f32) -> Result<State> {
    let endpoint = Endpoint::default(Flow::Render, Role::Console)?;
    let before = State {
        level: endpoint.level()?,
        muted: endpoint.muted()?,
    };
    let after = change.apply(Kind::Master, before, step_pct);

    if after.level != before.level {
        endpoint.set_level(after.level)?;
    }
    if after.muted != before.muted {
        endpoint.set_muted(after.muted)?;
    }
    Ok(after)
}

/// Applies `change` to the default microphone, and returns what the devices
/// say afterwards, not what was asked for: someone who believes they are
/// silent in a call must be told the truth. A mute reaches the default
/// communications microphone too when it is another device.
pub fn apply_to_microphone(change: Change, step_pct: f32) -> Result<State> {
    let microphones = Microphones::resolve()?;
    let before = microphones.state()?;
    let wanted = change.apply(Kind::Mic, before, step_pct);

    if wanted.level != before.level {
        microphones.set_level(wanted.level)?;
    }
    if wanted.muted != before.muted {
        microphones.set_muted(wanted.muted)?;
    }
    microphones.state()
}

/// The sessions, their applications, and the one behind the focused or the
/// pointed window.
///
/// Window to process to executable to sessions, in the order the design gives:
/// a Store app's frame is resolved to its core window's process first; the
/// executable path folds a browser's audio helper into its main process; the
/// process tree is walked only when the path matched nothing.
fn locate(target: Target) -> Option<(Sessions, Vec<Application>, usize)> {
    let window = match target {
        Target::Focused => y_core::target::focused(),
        Target::Pointed => y_core::target::pointed(),
        Target::Master | Target::Mic => None,
    }?;

    let mut pid = y_core::process::window_process_id(window);
    if let Some(app_pid) = y_core::process::uwp_app_pid(window) {
        pid = app_pid;
    }
    let process = WindowProcess {
        pid,
        exe: y_core::process::executable_path(pid),
    };

    let sessions = Sessions::snapshot().ok()?;
    let applications = grouping::group(sessions.infos());
    let found = grouping::for_window(
        &applications,
        sessions.infos(),
        &process,
        &std::collections::HashMap::new(),
    )
    .or_else(|| {
        let parents = y_core::process::parent_pids();
        grouping::for_window(&applications, sessions.infos(), &process, &parents)
    })?;
    Some((sessions, applications, found))
}

/// Applies `change` to the application behind the focused or the pointed
/// window and returns what to show, or `None` when that window has no audio
/// (or there is no eligible window).
pub fn apply_to_application(
    target: Target,
    change: Change,
    step_pct: f32,
) -> Option<(RowId, AppRef, State)> {
    let (sessions, applications, found) = locate(target)?;
    let application = &applications[found];
    let state = write_application(&sessions, application, change, step_pct);
    Some((
        RowId::App(application.key.clone()),
        app_ref(application),
        state,
    ))
}

/// Writes `change` to every session of `application` and returns what they
/// hold after, by the same aggregation as the row: the loudest level, muted
/// only if all are.
fn write_application(
    sessions: &Sessions,
    application: &Application,
    change: Change,
    step_pct: f32,
) -> State {
    let writes = grouping::apply(application, sessions.infos(), change, step_pct);
    for &(index, state) in &writes {
        // One session gone between the snapshot and now must not stop the rest.
        let _ = sessions.write(index, state);
    }
    State {
        level: writes.iter().map(|(_, s)| s.level).fold(0.0, f32::max),
        muted: writes.iter().all(|(_, s)| s.muted),
    }
}

/// How an application row is named and drawn.
fn app_ref(application: &Application) -> AppRef {
    let name = if application.is_system() {
        "System sounds".to_owned()
    } else if application.display_name.trim().is_empty() {
        "Application".to_owned()
    } else {
        application.display_name.clone()
    };
    AppRef {
        exe: application.exe.clone(),
        name,
    }
}
