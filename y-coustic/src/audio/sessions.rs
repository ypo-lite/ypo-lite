//! The audio sessions of every active output device, and writing their volume.
//!
//! A [`Sessions`] is a snapshot, taken for one action and dropped right after,
//! the same shape as [`super::Endpoint`]: nothing is cached, so a session that
//! appeared (an application began to play) or went away needs no notification
//! code. Everything COM about a session stays in this file; the rest of the
//! program sees [`SessionInfo`] and indices.
//!
//! Every active render endpoint is read, not only the default one: an
//! application can be routed to another output device in Windows' settings, and
//! must still be reached.

use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::{
    AudioSessionStateExpired, DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::core::{Interface, Result};

use y_coustic::model::{Level, State};

use super::EVENT_CONTEXT;
use super::grouping::SessionInfo;

/// What a session is, and the means to write to it.
pub struct Sessions {
    infos: Vec<SessionInfo>,
    /// The same order as `infos`.
    volumes: Vec<ISimpleAudioVolume>,
}

impl Sessions {
    /// Reads the sessions of every active output device. A device or a
    /// session that cannot be read is skipped, not fatal: one misbehaving
    /// driver must not take the others' volume away. Fails only when the
    /// devices cannot be listed at all.
    pub fn snapshot() -> Result<Self> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let devices = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };

        let mut sessions = Self {
            infos: Vec::new(),
            volumes: Vec::new(),
        };
        for index in 0..unsafe { devices.GetCount()? } {
            let Ok(device) = (unsafe { devices.Item(index) }) else {
                continue;
            };
            let Ok(manager) = (unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) })
            else {
                continue;
            };
            let Ok(list) = (unsafe { manager.GetSessionEnumerator() }) else {
                continue;
            };
            for position in 0..unsafe { list.GetCount() }.unwrap_or(0) {
                if let Ok(session) = unsafe { list.GetSession(position) }
                    && let Some((info, volume)) = read(&session)
                {
                    sessions.infos.push(info);
                    sessions.volumes.push(volume);
                }
            }
        }
        Ok(sessions)
    }

    pub fn infos(&self) -> &[SessionInfo] {
        &self.infos
    }

    /// Writes `state` to session `index`, only what differs from what the
    /// snapshot read. The write carries [`EVENT_CONTEXT`]. A session that has
    /// meanwhile gone fails its own write and nothing else.
    pub fn write(&self, index: usize, state: State) -> Result<()> {
        let (Some(info), Some(volume)) = (self.infos.get(index), self.volumes.get(index)) else {
            return Ok(());
        };
        let level: Level = state.level.clamp(0.0, 1.0);
        let level_result = if level != info.level {
            unsafe { volume.SetMasterVolume(level, &EVENT_CONTEXT) }
        } else {
            Ok(())
        };
        let mute_result = if state.muted != info.muted {
            unsafe { volume.SetMute(state.muted, &EVENT_CONTEXT) }
        } else {
            Ok(())
        };
        level_result.and(mute_result)
    }
}

/// One session's facts and its volume interface. `None` when it cannot be
/// read.
fn read(
    control: &windows::Win32::Media::Audio::IAudioSessionControl,
) -> Option<(SessionInfo, ISimpleAudioVolume)> {
    let details = control.cast::<IAudioSessionControl2>().ok()?;
    let volume = control.cast::<ISimpleAudioVolume>().ok()?;

    // A session of several processes answers with a success code that is not
    // `S_OK` (`AUDCLNT_S_NO_SINGLE_PROCESS`), and a pid of 0 either way, which
    // [`super::grouping`] already treats as a row of its own.
    let pid = unsafe { details.GetProcessId() }.unwrap_or(0);
    let is_system = unsafe { details.IsSystemSoundsSession() } == S_OK;
    let expired = unsafe { control.GetState() }.is_ok_and(|state| state == AudioSessionStateExpired);

    let display_name = unsafe { control.GetDisplayName() }
        .ok()
        .map(|raw| {
            let text = unsafe { raw.to_string() }.unwrap_or_default();
            unsafe { CoTaskMemFree(Some(raw.0 as *const _)) };
            text
        })
        .unwrap_or_default();

    let exe = (pid != 0 && !is_system)
        .then(|| y_core::process::executable_path(pid))
        .flatten();

    let info = SessionInfo {
        pid,
        exe,
        is_system,
        display_name,
        level: unsafe { volume.GetMasterVolume() }.ok()?,
        muted: unsafe { volume.GetMute() }.ok()?.as_bool(),
        expired,
    };
    Some((info, volume))
}
