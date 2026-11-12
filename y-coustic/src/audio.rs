//! Core Audio: the only COM in y-coustic.
//!
//! Every object here lives and dies on the message-loop thread, in the
//! single-threaded apartment [`init`] enters (no `Send` wrappers, no worker
//! thread). An [`Endpoint`] is resolved per action and dropped right after:
//! nothing is kept, so a changed default device is picked up with no
//! notification plumbing (the watchers of Phase 2b are for *showing* outside
//! changes, not for driving).

pub mod grouping;
pub mod sessions;
pub mod watch;

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    EDataFlow, ERole, IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eCommunications, eConsole,
    eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::core::{GUID, Result};

use y_coustic::model::{Level, State};

/// Passed with every write, so the watchers of Phase 2b can tell our own
/// changes from outside ones (`OnNotify` hands it back as `guidEventContext`).
pub const EVENT_CONTEXT: GUID = GUID::from_u128(0x8c0f_5d2e_41b7_4a63_9e1d_7b3a_c5f2_04d9);

/// Which side of the audio stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Speakers and headphones.
    Render,
    /// Microphones.
    Capture,
}

impl Flow {
    /// The flow as a message `wParam`.
    pub fn to_wparam(self) -> usize {
        match self {
            Self::Render => 0,
            Self::Capture => 1,
        }
    }

    pub fn from_wparam(value: usize) -> Option<Self> {
        match value {
            0 => Some(Self::Render),
            1 => Some(Self::Capture),
            _ => None,
        }
    }

    fn raw(self) -> EDataFlow {
        match self {
            Self::Render => eRender,
            Self::Capture => eCapture,
        }
    }
}

/// Which of the default devices Windows keeps per flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Games, music, system sounds.
    Console,
    /// Calls; often a different device from [`Role::Console`].
    Communications,
}

impl Role {
    fn raw(self) -> ERole {
        match self {
            Self::Console => eConsole,
            Self::Communications => eCommunications,
        }
    }
}

/// Enters the apartment this module's objects live in. Call once, on the
/// message-loop thread, before any other function here.
pub fn init() -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok()
}

/// The volume and mute of one default device.
pub struct Endpoint {
    volume: IAudioEndpointVolume,
    /// Windows' identifier of the device, to tell two defaults apart.
    id: String,
}

impl Endpoint {
    /// The current default device for `flow` and `role`. Fails when there is
    /// none (no microphone, every output disabled).
    pub fn default(flow: Flow, role: Role) -> Result<Self> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(flow.raw(), role.raw())? };
        let volume = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let id = unsafe {
            let raw = device.GetId()?;
            let id = raw.to_string().unwrap_or_default();
            CoTaskMemFree(Some(raw.0 as *const _));
            id
        };
        Ok(Self { volume, id })
    }

    pub fn level(&self) -> Result<Level> {
        unsafe { self.volume.GetMasterVolumeLevelScalar() }
    }

    pub fn set_level(&self, level: Level) -> Result<()> {
        unsafe {
            self.volume
                .SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), &EVENT_CONTEXT)
        }
    }

    pub fn muted(&self) -> Result<bool> {
        Ok(unsafe { self.volume.GetMute()? }.as_bool())
    }

    pub fn set_muted(&self, muted: bool) -> Result<()> {
        unsafe { self.volume.SetMute(muted, &EVENT_CONTEXT) }
    }
}

/// The default microphone, and the default *communications* microphone when
/// it is another device: a call application may be listening to the one the
/// user did not think of, so a mute must reach both.
pub struct Microphones {
    console: Endpoint,
    communications: Option<Endpoint>,
}

impl Microphones {
    /// Fails when there is no default microphone (none plugged in, or every
    /// one disabled).
    pub fn resolve() -> Result<Self> {
        let console = Endpoint::default(Flow::Capture, Role::Console)?;
        let communications = Endpoint::default(Flow::Capture, Role::Communications)
            .ok()
            .filter(|other| other.id != console.id);
        Ok(Self {
            console,
            communications,
        })
    }

    /// The level of the default microphone, and whether *every* device we
    /// drive is muted: a half-muted pair reads as live, since that is what
    /// someone in a call needs to hear about.
    pub fn state(&self) -> Result<State> {
        let mut muted = self.console.muted()?;
        if let Some(other) = &self.communications {
            muted &= other.muted().unwrap_or(false);
        }
        Ok(State {
            level: self.console.level()?,
            muted,
        })
    }

    /// Only the default microphone's level: the communications one keeps its
    /// own.
    pub fn set_level(&self, level: Level) -> Result<()> {
        self.console.set_level(level)
    }

    /// Sets the mute on every device we drive. The caller reads [`state`]
    /// back afterwards: what a user is told must be what the device says, not
    /// what was asked for.
    ///
    /// [`state`]: Microphones::state
    pub fn set_muted(&self, muted: bool) -> Result<()> {
        let first = self.console.set_muted(muted);
        let second = self
            .communications
            .as_ref()
            .map_or(Ok(()), |other| other.set_muted(muted));
        first.and(second)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads and rewrites the same level and mute, so it changes nothing the
    /// user can hear. Needs an output device: `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs an audio output device"]
    fn the_default_output_reads_and_writes() {
        init().unwrap();
        let endpoint = Endpoint::default(Flow::Render, Role::Console).unwrap();
        let level = endpoint.level().unwrap();
        let muted = endpoint.muted().unwrap();
        endpoint.set_level(level).unwrap();
        endpoint.set_muted(muted).unwrap();
        assert!((endpoint.level().unwrap() - level).abs() < 0.01);
        assert_eq!(endpoint.muted().unwrap(), muted);
    }
}
