//! Watchers: learning that the volume or the mute changed from outside.
//!
//! Two COM callbacks, both reduced to the one thing a callback may safely do:
//! `PostMessageW` to the owner window, with the flow as `wParam`. The reaction
//! (re-reading the device, the tray icon, the HUD) happens on the message loop,
//! on the thread every other audio object lives on.
//!
//! * [`EndpointWatch`]: an `IAudioEndpointVolumeCallback` on a default
//!   endpoint. Core Audio calls `OnNotify` from a thread of its own, not ours:
//!   the callback holds nothing but a window handle and a flow, and compares
//!   the event context with [`EVENT_CONTEXT`] so our own writes post nothing.
//! * [`DeviceWatch`]: an `IMMNotificationClient`, to learn that the default
//!   device changed, which moves the `EndpointWatch` to the new one.
//!
//! Both owners **unregister first, then release**, in `Drop`: releasing a
//! callback that is still registered leaves the audio service holding a
//! dangling pointer.
//!
//! Needs `windows-core` as a direct dependency: `#[implement]` emits
//! `::windows_core::` paths.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator,
    IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, eCapture, eConsole,
    eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{PCWSTR, Result, implement};

use y_core::ids::WM_APP_FIRST;

use super::{EVENT_CONTEXT, Endpoint, Flow, Role};

/// Posted when a watched endpoint's volume or mute changed from outside.
/// `wParam` is the [`Flow`] as [`Flow::to_wparam`] encodes it.
pub const WM_ENDPOINT_CHANGED: u32 = WM_APP_FIRST;

/// Posted when the default device of a flow changed. Same `wParam`.
pub const WM_DEFAULT_DEVICE_CHANGED: u32 = WM_APP_FIRST + 1;

fn post(owner: isize, message: u32, flow: Flow) {
    // Fails only when the window is gone, which is the end of the process.
    let _ = unsafe {
        PostMessageW(
            Some(HWND(owner as *mut _)),
            message,
            WPARAM(flow.to_wparam()),
            LPARAM(0),
        )
    };
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback {
    owner: isize,
    flow: Flow,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, notify: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        let ours =
            unsafe { notify.as_ref() }.is_some_and(|data| data.guidEventContext == EVENT_CONTEXT);
        if !ours {
            post(self.owner, WM_ENDPOINT_CHANGED, self.flow);
        }
        Ok(())
    }
}

/// A registered volume callback on the default endpoint of one flow.
pub struct EndpointWatch {
    volume: IAudioEndpointVolume,
    callback: IAudioEndpointVolumeCallback,
}

impl EndpointWatch {
    /// Fails when there is no default endpoint for the flow (no microphone).
    pub fn new(owner: HWND, flow: Flow) -> Result<Self> {
        let endpoint = Endpoint::default(flow, Role::Console)?;
        let callback: IAudioEndpointVolumeCallback = VolumeCallback {
            owner: owner.0 as isize,
            flow,
        }
        .into();
        unsafe { endpoint.volume.RegisterControlChangeNotify(&callback)? };
        Ok(Self {
            volume: endpoint.volume,
            callback,
        })
    }
}

impl Drop for EndpointWatch {
    fn drop(&mut self) {
        // Unregister first; the fields are released after this returns.
        let _ = unsafe { self.volume.UnregisterControlChangeNotify(&self.callback) };
    }
}

#[implement(IMMNotificationClient)]
struct DeviceCallback {
    owner: isize,
}

impl IMMNotificationClient_Impl for DeviceCallback_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        // One message per change: the console role is the one we drive.
        if role == eConsole {
            match flow {
                f if f == eRender => post(self.owner, WM_DEFAULT_DEVICE_CHANGED, Flow::Render),
                f if f == eCapture => post(self.owner, WM_DEFAULT_DEVICE_CHANGED, Flow::Capture),
                _ => {}
            }
        }
        Ok(())
    }

    fn OnPropertyValueChanged(
        &self,
        _: &PCWSTR,
        _: &windows::Win32::Foundation::PROPERTYKEY,
    ) -> Result<()> {
        Ok(())
    }
}

/// A registered device-change client.
pub struct DeviceWatch {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
}

impl DeviceWatch {
    pub fn new(owner: HWND) -> Result<Self> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
        let client: IMMNotificationClient = DeviceCallback {
            owner: owner.0 as isize,
        }
        .into();
        unsafe { enumerator.RegisterEndpointNotificationCallback(&client)? };
        Ok(Self { enumerator, client })
    }
}

impl Drop for DeviceWatch {
    fn drop(&mut self) {
        let _ = unsafe {
            self.enumerator
                .UnregisterEndpointNotificationCallback(&self.client)
        };
    }
}

/// Every watcher the configuration asks for. Nothing is resident for a flow
/// that is not in the list, and with an empty list not even the device client.
pub struct Watchers {
    owner: HWND,
    endpoints: Vec<(Flow, Option<EndpointWatch>)>,
    /// Kept alive for as long as any endpoint is watched.
    _devices: Option<DeviceWatch>,
}

impl Watchers {
    /// Watches the default endpoint of each of `flows`. A flow with no device
    /// right now is retried when the default device changes.
    pub fn new(owner: HWND, flows: &[Flow]) -> Self {
        let endpoints = flows
            .iter()
            .map(|&flow| (flow, EndpointWatch::new(owner, flow).ok()))
            .collect();
        let devices = if flows.is_empty() {
            None
        } else {
            DeviceWatch::new(owner).ok()
        };
        Self {
            owner,
            endpoints,
            _devices: devices,
        }
    }

    /// The default device of `flow` changed: move the watcher onto the new one.
    /// The old one is unregistered and released before the new one exists.
    pub fn rewatch(&mut self, flow: Flow) {
        let owner = self.owner;
        for (watched, watch) in &mut self.endpoints {
            if *watched == flow {
                *watch = None;
                *watch = EndpointWatch::new(owner, flow).ok();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio;

    #[test]
    fn the_flow_survives_the_trip_through_a_message() {
        for flow in [Flow::Render, Flow::Capture] {
            assert_eq!(Flow::from_wparam(flow.to_wparam()), Some(flow));
        }
        assert_eq!(Flow::from_wparam(7), None);
    }

    /// Registers and releases both watchers, which proves the callbacks
    /// implement their interfaces and that the unregister-then-release order
    /// does not crash. Needs an output device: `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs an audio output device"]
    fn the_watchers_register_and_release() {
        audio::init().unwrap();
        let owner = HWND(std::ptr::null_mut());
        let mut watchers = Watchers::new(owner, &[Flow::Render]);
        assert!(watchers.endpoints[0].1.is_some());
        watchers.rewatch(Flow::Render);
        assert!(watchers.endpoints[0].1.is_some());
        drop(watchers);
    }
}
