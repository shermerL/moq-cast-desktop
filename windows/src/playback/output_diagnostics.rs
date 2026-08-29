//! Best-effort Windows output-device diagnostics for remote audio playback.

use std::{collections::hash_map::RandomState, hash::BuildHasher, sync::OnceLock};

const MAX_ENDPOINT_NAME_CHARS: usize = 96;

fn redact_endpoint_identity(identity: &str) -> String {
    static HASHER: OnceLock<RandomState> = OnceLock::new();

    let hash = HASHER.get_or_init(RandomState::new).hash_one(identity);
    format!("endpoint-{hash:016x}")
}

fn sanitize_endpoint_name(name: &str, current_username: Option<&str>) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "unavailable".to_owned();
    }

    let lowercase = trimmed.to_ascii_lowercase();
    if trimmed.starts_with('/')
        || trimmed.starts_with("\\\\")
        || lowercase.contains(":\\")
        || lowercase.contains("\\users\\")
        || lowercase.contains("/users/")
        || lowercase.contains("/home/")
    {
        return "redacted-path".to_owned();
    }
    if current_username.is_some_and(|username| {
        !username.is_empty() && lowercase.contains(&username.to_ascii_lowercase())
    }) {
        return "redacted-user-name".to_owned();
    }
    if contains_guid(trimmed) {
        return "redacted-identifier".to_owned();
    }

    let mut sanitized = String::new();
    let mut previous_space = false;
    for character in trimmed.chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if character.is_whitespace() {
            if !previous_space && !sanitized.is_empty() {
                sanitized.push(' ');
            }
            previous_space = true;
        } else {
            sanitized.push(character);
            previous_space = false;
        }
        if sanitized.chars().count() >= MAX_ENDPOINT_NAME_CHARS {
            break;
        }
    }
    sanitized.trim().to_owned()
}

fn contains_guid(value: &str) -> bool {
    value.as_bytes().windows(36).any(|candidate| {
        candidate.iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23) && *byte == b'-'
                || !matches!(index, 8 | 13 | 18 | 23) && byte.is_ascii_hexdigit()
        })
    })
}

#[cfg(target_os = "windows")]
pub(super) fn spawn(stage: &'static str, view_generation: u64, elapsed_ms: u64) {
    let spawn = std::thread::Builder::new()
        .name("moqcast-audio-diagnostics".to_owned())
        .spawn(move || platform::collect_and_log(stage, view_generation, elapsed_ms));
    if let Err(error) = spawn {
        tracing::warn!(
            stage,
            view_generation,
            elapsed_ms,
            error_kind = ?error.kind(),
            "could not start Windows audio diagnostics; playback continues"
        );
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, slice};

    use windows::{
        Win32::{
            Foundation::{ERROR_NO_UNICODE_TRANSLATION, PROPERTYKEY},
            Media::Audio::{
                AudioSessionState, AudioSessionStateActive, AudioSessionStateExpired,
                AudioSessionStateInactive, DEVICE_STATE, DEVICE_STATE_ACTIVE,
                DEVICE_STATE_DISABLED, DEVICE_STATE_NOTPRESENT, DEVICE_STATE_UNPLUGGED,
                Endpoints::IAudioEndpointVolume, IAudioSessionControl, IAudioSessionControl2,
                IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume,
                MMDeviceEnumerator, eConsole, eMultimedia, eRender,
            },
            System::{
                Com::{
                    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                    CoTaskMemFree, CoUninitialize, STGM_READ,
                    StructuredStorage::{PROPVARIANT, PropVariantClear},
                },
                Variant::VT_LPWSTR,
            },
        },
        core::{GUID, Interface, PWSTR},
    };

    use super::{redact_endpoint_identity, sanitize_endpoint_name};

    const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

    pub(super) fn collect_and_log(stage: &'static str, view_generation: u64, elapsed_ms: u64) {
        let apartment = match ComApartment::initialize() {
            Ok(apartment) => apartment,
            Err(error) => {
                log_failure(stage, view_generation, elapsed_ms, "com_initialize", error);
                return;
            }
        };

        let snapshot = unsafe { collect() };
        drop(apartment);
        match snapshot {
            Ok(snapshot) => snapshot.log(stage, view_generation, elapsed_ms),
            Err((query, error)) => log_failure(stage, view_generation, elapsed_ms, query, error),
        }
    }

    fn log_failure(
        stage: &'static str,
        view_generation: u64,
        elapsed_ms: u64,
        query: &'static str,
        error: windows::core::Error,
    ) {
        tracing::warn!(
            stage,
            view_generation,
            elapsed_ms,
            query,
            hresult = error.code().0,
            "Windows audio diagnostics unavailable; playback continues"
        );
    }

    struct ComApartment;

    impl ComApartment {
        fn initialize() -> windows::core::Result<Self> {
            let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            result.ok().map(|()| Self)
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    struct Snapshot {
        endpoint_identity: String,
        endpoint_name: String,
        endpoint_state: &'static str,
        console_multimedia_same: Option<bool>,
        endpoint_muted: Option<bool>,
        endpoint_volume_percent: Option<f32>,
        session: SessionSnapshot,
    }

    impl Snapshot {
        fn log(&self, stage: &'static str, view_generation: u64, elapsed_ms: u64) {
            tracing::info!(
                stage,
                view_generation,
                elapsed_ms,
                endpoint_identity = %self.endpoint_identity,
                endpoint_name = %self.endpoint_name,
                endpoint_state = self.endpoint_state,
                console_multimedia_same = ?self.console_multimedia_same,
                endpoint_muted = ?self.endpoint_muted,
                endpoint_volume_percent = ?self.endpoint_volume_percent,
                audio_session_association = self.session.association,
                audio_session_volume_query = self.session.volume_query,
                audio_session_matches = self.session.matches,
                audio_session_state = ?self.session.state,
                audio_session_muted = ?self.session.muted,
                audio_session_volume_percent = ?self.session.volume_percent,
                "Windows audio output diagnostics; audible output is not proven"
            );
        }
    }

    struct SessionSnapshot {
        association: &'static str,
        volume_query: &'static str,
        matches: usize,
        state: Option<&'static str>,
        muted: Option<bool>,
        volume_percent: Option<f32>,
    }

    unsafe fn collect() -> Result<Snapshot, (&'static str, windows::core::Error)> {
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|error| ("device_enumerator", error))?
        };
        let console = unsafe {
            enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|error| ("default_console_endpoint", error))?
        };
        let console_endpoint_id = unsafe { endpoint_id(&console) }
            .map_err(|error| ("default_console_endpoint_id", error))?;
        let multimedia = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }.ok();
        let console_multimedia_same = multimedia.and_then(|device| {
            unsafe { endpoint_id(&device) }
                .ok()
                .map(|id| id == console_endpoint_id)
        });
        let endpoint_name = unsafe { endpoint_name(&console) }
            .map(|name| sanitize_endpoint_name(&name, std::env::var("USERNAME").ok().as_deref()))
            .unwrap_or_else(|| "unavailable".to_owned());
        let endpoint_state = unsafe { console.GetState() }
            .map(endpoint_state)
            .unwrap_or("unavailable");
        let endpoint_volume: Option<IAudioEndpointVolume> =
            unsafe { console.Activate(CLSCTX_ALL, None) }.ok();
        let endpoint_muted = endpoint_volume
            .as_ref()
            .and_then(|volume| unsafe { volume.GetMute() }.ok())
            .map(|muted| muted.as_bool());
        let endpoint_volume_percent = endpoint_volume
            .as_ref()
            .and_then(|volume| unsafe { volume.GetMasterVolumeLevelScalar() }.ok())
            .map(percent);
        let session = unsafe { session_snapshot(&console) };

        Ok(Snapshot {
            endpoint_identity: redact_endpoint_identity(&console_endpoint_id),
            endpoint_name,
            endpoint_state,
            console_multimedia_same,
            endpoint_muted,
            endpoint_volume_percent,
            session,
        })
    }

    unsafe fn session_snapshot(endpoint: &IMMDevice) -> SessionSnapshot {
        let manager: IAudioSessionManager2 = match unsafe { endpoint.Activate(CLSCTX_ALL, None) } {
            Ok(manager) => manager,
            Err(_) => return SessionSnapshot::skipped("session_manager_unavailable", 0),
        };
        let enumerator = match unsafe { manager.GetSessionEnumerator() } {
            Ok(enumerator) => enumerator,
            Err(_) => return SessionSnapshot::skipped("session_enumerator_unavailable", 0),
        };
        let count = match unsafe { enumerator.GetCount() } {
            Ok(count) => count,
            Err(_) => return SessionSnapshot::skipped("session_count_unavailable", 0),
        };
        let process_id = std::process::id();
        let mut matching: Option<IAudioSessionControl> = None;
        let mut matches = 0usize;
        for index in 0..count {
            let Ok(control) = (unsafe { enumerator.GetSession(index) }) else {
                continue;
            };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            if unsafe { control2.GetProcessId() }.ok() == Some(process_id) {
                matches += 1;
                matching = Some(control);
            }
        }
        let Some(control) = matching.filter(|_| matches == 1) else {
            return SessionSnapshot::skipped(
                if matches == 0 {
                    "no_current_process_session"
                } else {
                    "ambiguous_current_process_sessions"
                },
                matches,
            );
        };

        let state = unsafe { control.GetState() }.ok().map(session_state);
        // CPAL initializes its WASAPI client with a null session GUID. Querying the
        // endpoint's default process session is only considered reliable after a
        // unique process-id match has been observed above.
        let volume: Option<ISimpleAudioVolume> =
            unsafe { manager.GetSimpleAudioVolume(None, 0) }.ok();
        let volume_query = if volume.is_some() {
            "matched_default_process_session"
        } else {
            "simple_volume_unavailable"
        };
        let muted = volume
            .as_ref()
            .and_then(|volume| unsafe { volume.GetMute() }.ok())
            .map(|muted| muted.as_bool());
        let volume_percent = volume
            .as_ref()
            .and_then(|volume| unsafe { volume.GetMasterVolume() }.ok())
            .map(percent);
        SessionSnapshot {
            association: "unique_current_process_session",
            volume_query,
            matches,
            state,
            muted,
            volume_percent,
        }
    }

    impl SessionSnapshot {
        fn skipped(association: &'static str, matches: usize) -> Self {
            Self {
                association,
                volume_query: "skipped_without_unique_session",
                matches,
                state: None,
                muted: None,
                volume_percent: None,
            }
        }
    }

    fn endpoint_state(state: DEVICE_STATE) -> &'static str {
        match state {
            DEVICE_STATE_ACTIVE => "active",
            DEVICE_STATE_DISABLED => "disabled",
            DEVICE_STATE_NOTPRESENT => "not_present",
            DEVICE_STATE_UNPLUGGED => "unplugged",
            _ => "unknown",
        }
    }

    fn session_state(state: AudioSessionState) -> &'static str {
        if state == AudioSessionStateActive {
            "active"
        } else if state == AudioSessionStateInactive {
            "inactive"
        } else if state == AudioSessionStateExpired {
            "expired"
        } else {
            "unknown"
        }
    }

    fn percent(value: f32) -> f32 {
        (value.clamp(0.0, 1.0) * 1_000.0).round() / 10.0
    }

    unsafe fn endpoint_id(device: &IMMDevice) -> windows::core::Result<String> {
        let value = OwnedPwstr(unsafe { device.GetId()? });
        unsafe { value.0.to_string() }.map_err(|error| {
            windows::core::Error::new(
                ERROR_NO_UNICODE_TRANSLATION.to_hresult(),
                format!("default audio endpoint ID contained invalid UTF-16: {error}"),
            )
        })
    }

    struct OwnedPwstr(PWSTR);

    impl Drop for OwnedPwstr {
        fn drop(&mut self) {
            unsafe { CoTaskMemFree(Some(self.0.as_ptr().cast())) };
        }
    }

    unsafe fn endpoint_name(device: &IMMDevice) -> Option<String> {
        let store = unsafe { device.OpenPropertyStore(STGM_READ) }.ok()?;
        let mut value = unsafe { store.GetValue(&FRIENDLY_NAME) }.ok()?;
        let result = unsafe { propvariant_string(&value) };
        unsafe { PropVariantClear(&mut value) }.ok();
        result
    }

    unsafe fn propvariant_string(value: &PROPVARIANT) -> Option<String> {
        let value = unsafe { &value.Anonymous.Anonymous };
        if value.vt != VT_LPWSTR {
            return None;
        }
        let pointer = unsafe { *(&value.Anonymous as *const _ as *const *const u16) };
        if pointer.is_null() {
            return None;
        }
        let mut length = 0usize;
        while length < 32_768 && unsafe { *pointer.add(length) } != 0 {
            length += 1;
        }
        if length == 32_768 {
            return None;
        }
        let wide = unsafe { slice::from_raw_parts(pointer, length) };
        Some(OsString::from_wide(wide).to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_identity_is_opaque_stable_within_process_and_input_sensitive() {
        let identity = "{0.0.0.00000000}.{12345678-1234-1234-1234-123456789abc}";
        let redacted = redact_endpoint_identity(identity);

        assert_eq!(redacted, redact_endpoint_identity(identity));
        assert_ne!(redacted, redact_endpoint_identity("another endpoint"));
        assert!(redacted.starts_with("endpoint-"));
        assert_eq!(redacted.len(), "endpoint-".len() + 16);
        assert!(!redacted.contains("12345678"));
        assert!(!redacted.contains('{'));
    }

    #[test]
    fn endpoint_name_removes_controls_and_bounds_length() {
        let name = format!("  Speakers\n\t{}  ", "x".repeat(120));
        let sanitized = sanitize_endpoint_name(&name, None);

        assert!(sanitized.starts_with("Speakers "));
        assert!(!sanitized.contains('\n'));
        assert!(!sanitized.contains('\t'));
        assert!(sanitized.chars().count() <= MAX_ENDPOINT_NAME_CHARS);
    }

    #[test]
    fn endpoint_name_redacts_paths_and_guid_like_values() {
        assert_eq!(
            sanitize_endpoint_name(r"C:\Users\private\endpoint", None),
            "redacted-path"
        );
        assert_eq!(
            sanitize_endpoint_name("Speakers {12345678-1234-1234-1234-123456789abc}", None,),
            "redacted-identifier"
        );
        assert_eq!(
            sanitize_endpoint_name("Alice's Headphones", Some("Alice")),
            "redacted-user-name"
        );
    }
}
