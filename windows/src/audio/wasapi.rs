//! Direct shared-mode WASAPI loopback over the default render endpoint.

use std::{
    ffi::c_void,
    io, ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use tokio::sync::mpsc;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::{
            Audio::{
                AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT,
                AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
                AUDCLNT_STREAMFLAGS_NOPERSIST, IAudioCaptureClient, IAudioClient, IMMDevice,
                IMMDeviceEnumerator, MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX,
                WAVEFORMATEXTENSIBLE, eMultimedia, eRender,
            },
            KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE},
            Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT},
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
                CoUninitialize,
            },
            Threading::{CreateEventW, WaitForSingleObject},
        },
    },
    core::{Error as WindowsError, PCWSTR},
};

use super::{AudioIssue, MixFormat, SampleEncoding};

const EVENT_TIMEOUT_MS: u32 = 100;
const DEVICE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const CAPTURE_QUEUE_CAPACITY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FailureKind {
    NoDefaultOutput,
    DefaultOutputChanged,
    DeviceInvalidated,
    UnsupportedMixFormat,
    Backend,
}

impl FailureKind {
    pub(super) fn recovery_issue(self) -> Option<AudioIssue> {
        match self {
            Self::NoDefaultOutput => Some(AudioIssue::NoDefaultOutput),
            Self::DefaultOutputChanged => Some(AudioIssue::DefaultOutputChanged),
            Self::DeviceInvalidated => Some(AudioIssue::DeviceInvalidated),
            Self::UnsupportedMixFormat | Self::Backend => None,
        }
    }

    pub(super) fn issue(self) -> AudioIssue {
        match self {
            Self::NoDefaultOutput => AudioIssue::NoDefaultOutput,
            Self::DefaultOutputChanged => AudioIssue::DefaultOutputChanged,
            Self::DeviceInvalidated => AudioIssue::DeviceInvalidated,
            Self::UnsupportedMixFormat => AudioIssue::UnsupportedMixFormat,
            Self::Backend => AudioIssue::CaptureBackend,
        }
    }
}

#[derive(Debug)]
pub(super) struct Failure {
    pub(super) kind: FailureKind,
    pub(super) detail: String,
}

impl Failure {
    fn windows(stage: &'static str, error: WindowsError) -> Self {
        let kind = if error.code() == AUDCLNT_E_DEVICE_INVALIDATED {
            FailureKind::DeviceInvalidated
        } else {
            FailureKind::Backend
        };
        Self {
            kind,
            detail: format!("{stage}: {error}"),
        }
    }

    fn backend(stage: &'static str, detail: impl std::fmt::Display) -> Self {
        Self {
            kind: FailureKind::Backend,
            detail: format!("{stage}: {detail}"),
        }
    }

    fn unsupported(detail: impl std::fmt::Display) -> Self {
        Self {
            kind: FailureKind::UnsupportedMixFormat,
            detail: detail.to_string(),
        }
    }
}

pub(super) struct CapturedPacket {
    pub(super) timestamp_us: u64,
    pub(super) samples: Vec<f32>,
    pub(super) silent: bool,
    pub(super) discontinuity: bool,
}

pub(super) enum CaptureEvent {
    Ready(MixFormat),
    Packet(CapturedPacket),
    Stopped(Failure),
}

pub(super) struct Capture {
    receiver: mpsc::Receiver<CaptureEvent>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    pub(super) fn start(clock: moq_mux::Clock) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel(CAPTURE_QUEUE_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("moqcast-wasapi-loopback".to_owned())
            .spawn(move || {
                if let Err(failure) = run(sender.clone(), thread_stop, clock) {
                    let _ = sender.try_send(CaptureEvent::Stopped(failure));
                }
            })?;
        Ok(Self {
            receiver,
            stop,
            thread: Some(thread),
        })
    }

    pub(super) async fn recv(&mut self) -> Option<CaptureEvent> {
        self.receiver.recv().await
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.close();
    }
}

fn run(
    sender: mpsc::Sender<CaptureEvent>,
    stop: Arc<AtomicBool>,
    clock: moq_mux::Clock,
) -> Result<(), Failure> {
    let _com = ComApartment::initialize()?;
    let mut stream = WasapiStream::open()?;
    if sender.try_send(CaptureEvent::Ready(stream.format)).is_err() {
        return Ok(());
    }

    let mut timeline = Timeline::default();
    let mut dropped_packet = false;
    let mut next_device_poll = Instant::now() + DEVICE_POLL_INTERVAL;

    while !stop.load(Ordering::Acquire) {
        let wait = unsafe { WaitForSingleObject(stream.event.0, EVENT_TIMEOUT_MS) };
        match wait {
            WAIT_OBJECT_0 => {
                for mut packet in stream.drain(&mut timeline, clock)? {
                    packet.discontinuity |= dropped_packet;
                    match sender.try_send(CaptureEvent::Packet(packet)) {
                        Ok(()) => dropped_packet = false,
                        Err(mpsc::error::TrySendError::Full(_)) => dropped_packet = true,
                        Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
                    }
                }
            }
            WAIT_TIMEOUT => {}
            WAIT_FAILED => {
                return Err(Failure::windows(
                    "WaitForSingleObject",
                    WindowsError::from_win32(),
                ));
            }
            other => {
                return Err(Failure::backend(
                    "WaitForSingleObject",
                    format_args!("unexpected wait result {}", other.0),
                ));
            }
        }

        if Instant::now() >= next_device_poll {
            let current = default_device_id(&stream.enumerator)?;
            if current != stream.device_id {
                return Err(Failure {
                    kind: FailureKind::DefaultOutputChanged,
                    detail: "the default render endpoint changed".to_owned(),
                });
            }
            next_device_poll = Instant::now() + DEVICE_POLL_INTERVAL;
        }
    }

    Ok(())
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, Failure> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        result
            .ok()
            .map_err(|error| Failure::windows("CoInitializeEx", error))?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct TaskMemory<T>(*mut T);

impl<T> TaskMemory<T> {
    fn as_ptr(&self) -> *mut T {
        self.0
    }
}

impl<T> Drop for TaskMemory<T> {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.cast::<c_void>())) };
    }
}

struct WasapiStream {
    enumerator: IMMDeviceEnumerator,
    device_id: String,
    audio_client: IAudioClient,
    capture_client: IAudioCaptureClient,
    event: OwnedHandle,
    format: MixFormat,
}

impl WasapiStream {
    fn open() -> Result<Self, Failure> {
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|error| Failure::windows("CoCreateInstance(MMDeviceEnumerator)", error))?
        };
        let device = default_device(&enumerator)?;
        let device_id = device_id(&device)?;
        let audio_client: IAudioClient = unsafe {
            device
                .Activate(CLSCTX_ALL, None)
                .map_err(|error| Failure::windows("IMMDevice::Activate(IAudioClient)", error))?
        };
        let mix = TaskMemory(unsafe {
            audio_client
                .GetMixFormat()
                .map_err(|error| Failure::windows("IAudioClient::GetMixFormat", error))?
        });
        let format = parse_mix_format(mix.as_ptr())?;
        let flags = AUDCLNT_STREAMFLAGS_LOOPBACK
            | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_NOPERSIST;
        unsafe {
            audio_client
                .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, mix.as_ptr(), None)
                .map_err(|error| Failure::windows("IAudioClient::Initialize(loopback)", error))?;
        }

        let event = OwnedHandle(unsafe {
            CreateEventW(None, false, false, PCWSTR::null())
                .map_err(|error| Failure::windows("CreateEventW", error))?
        });
        unsafe {
            audio_client
                .SetEventHandle(event.0)
                .map_err(|error| Failure::windows("IAudioClient::SetEventHandle", error))?;
        }
        let capture_client = unsafe {
            audio_client
                .GetService::<IAudioCaptureClient>()
                .map_err(|error| Failure::windows("IAudioClient::GetService", error))?
        };
        unsafe {
            audio_client
                .Start()
                .map_err(|error| Failure::windows("IAudioClient::Start", error))?;
        }

        Ok(Self {
            enumerator,
            device_id,
            audio_client,
            capture_client,
            event,
            format,
        })
    }

    fn drain(
        &mut self,
        timeline: &mut Timeline,
        clock: moq_mux::Clock,
    ) -> Result<Vec<CapturedPacket>, Failure> {
        let mut packets = Vec::new();
        loop {
            let frames = unsafe {
                self.capture_client.GetNextPacketSize().map_err(|error| {
                    Failure::windows("IAudioCaptureClient::GetNextPacketSize", error)
                })?
            };
            if frames == 0 {
                return Ok(packets);
            }

            let mut data = ptr::null_mut();
            let mut read_frames = 0;
            let mut flags = 0;
            let mut qpc_position = 0;
            unsafe {
                self.capture_client
                    .GetBuffer(
                        &mut data,
                        &mut read_frames,
                        &mut flags,
                        None,
                        Some(&mut qpc_position),
                    )
                    .map_err(|error| Failure::windows("IAudioCaptureClient::GetBuffer", error))?;
            }

            let discontinuity = flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0;
            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            let packet = self.packet(
                data,
                read_frames,
                qpc_position,
                silent,
                discontinuity,
                timeline,
                clock,
            );
            let release = unsafe { self.capture_client.ReleaseBuffer(read_frames) }
                .map_err(|error| Failure::windows("IAudioCaptureClient::ReleaseBuffer", error));
            match (packet, release) {
                (Ok(packet), Ok(())) => packets.push(packet),
                (Err(error), _) | (_, Err(error)) => return Err(error),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn packet(
        &self,
        data: *mut u8,
        frames: u32,
        qpc_position: u64,
        silent: bool,
        discontinuity: bool,
        timeline: &mut Timeline,
        clock: moq_mux::Clock,
    ) -> Result<CapturedPacket, Failure> {
        let sample_count = usize::try_from(frames)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(self.format.channels)))
            .ok_or_else(|| Failure::backend("WASAPI packet", "sample count overflow"))?;
        let samples = if silent {
            vec![0.0; sample_count]
        } else {
            if data.is_null() {
                return Err(Failure::backend(
                    "WASAPI packet",
                    "non-silent buffer contained no data",
                ));
            }
            let bytes = usize::try_from(frames)
                .ok()
                .and_then(|frames| frames.checked_mul(usize::from(self.format.block_align)))
                .ok_or_else(|| Failure::backend("WASAPI packet", "byte count overflow"))?;
            let packet = unsafe { std::slice::from_raw_parts(data, bytes) };
            self.format
                .decode(packet, frames)
                .map_err(|error| Failure::backend("decode WASAPI PCM", error))?
        };

        Ok(CapturedPacket {
            timestamp_us: timeline.timestamp(qpc_position, clock, discontinuity),
            samples,
            silent,
            discontinuity,
        })
    }
}

impl Drop for WasapiStream {
    fn drop(&mut self) {
        let _ = unsafe { self.audio_client.Stop() };
    }
}

#[derive(Default)]
struct Timeline {
    qpc_anchor: Option<(u64, u64)>,
}

impl Timeline {
    fn timestamp(
        &mut self,
        qpc_position_100ns: u64,
        clock: moq_mux::Clock,
        discontinuity: bool,
    ) -> u64 {
        let now = clock.micros();
        if discontinuity || qpc_position_100ns == 0 {
            self.qpc_anchor = None;
        }
        let (anchor_qpc, anchor_us) = *self.qpc_anchor.get_or_insert((qpc_position_100ns, now));
        qpc_position_100ns
            .checked_sub(anchor_qpc)
            .map(|delta| anchor_us.saturating_add(delta / 10))
            .unwrap_or(now)
    }
}

fn default_device(enumerator: &IMMDeviceEnumerator) -> Result<IMMDevice, Failure> {
    unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) }.map_err(|error| Failure {
        kind: FailureKind::NoDefaultOutput,
        detail: format!("GetDefaultAudioEndpoint: {error}"),
    })
}

fn default_device_id(enumerator: &IMMDeviceEnumerator) -> Result<String, Failure> {
    let device = default_device(enumerator)?;
    device_id(&device)
}

fn device_id(device: &IMMDevice) -> Result<String, Failure> {
    let id = unsafe {
        device
            .GetId()
            .map_err(|error| Failure::windows("IMMDevice::GetId", error))?
    };
    let decoded = unsafe { id.to_string() };
    unsafe { CoTaskMemFree(Some(id.0.cast::<c_void>())) };
    decoded.map_err(|error| Failure::backend("decode IMMDevice id", error))
}

fn parse_mix_format(format: *const WAVEFORMATEX) -> Result<MixFormat, Failure> {
    if format.is_null() {
        return Err(Failure::unsupported(
            "IAudioClient returned a null mix format",
        ));
    }
    let tag = u32::from(unsafe { ptr::addr_of!((*format).wFormatTag).read_unaligned() });
    let channels = unsafe { ptr::addr_of!((*format).nChannels).read_unaligned() };
    let sample_rate = unsafe { ptr::addr_of!((*format).nSamplesPerSec).read_unaligned() };
    let block_align = unsafe { ptr::addr_of!((*format).nBlockAlign).read_unaligned() };
    let bits = unsafe { ptr::addr_of!((*format).wBitsPerSample).read_unaligned() };
    let extra_size = unsafe { ptr::addr_of!((*format).cbSize).read_unaligned() };

    let encoding = if tag == WAVE_FORMAT_IEEE_FLOAT {
        if bits != 32 {
            return Err(Failure::unsupported(format_args!(
                "unsupported {bits}-bit IEEE float mix format"
            )));
        }
        SampleEncoding::Float32
    } else if tag == WAVE_FORMAT_PCM {
        pcm_encoding(bits, bits)?
    } else if tag == WAVE_FORMAT_EXTENSIBLE {
        if usize::from(extra_size)
            < std::mem::size_of::<WAVEFORMATEXTENSIBLE>() - std::mem::size_of::<WAVEFORMATEX>()
        {
            return Err(Failure::unsupported("truncated WAVEFORMATEXTENSIBLE"));
        }
        let extensible = unsafe { ptr::read_unaligned(format.cast::<WAVEFORMATEXTENSIBLE>()) };
        let samples = unsafe { ptr::addr_of!(extensible.Samples).read_unaligned() };
        let valid_bits = unsafe { samples.wValidBitsPerSample };
        let sub_format = unsafe { ptr::addr_of!(extensible.SubFormat).read_unaligned() };
        if sub_format == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
            if bits != 32 || valid_bits != 32 {
                return Err(Failure::unsupported(format_args!(
                    "unsupported extensible float mix format: {valid_bits}/{bits} bits"
                )));
            }
            SampleEncoding::Float32
        } else if sub_format == KSDATAFORMAT_SUBTYPE_PCM {
            pcm_encoding(bits, valid_bits)?
        } else {
            return Err(Failure::unsupported("unsupported extensible audio subtype"));
        }
    } else {
        return Err(Failure::unsupported(format_args!(
            "unsupported wave format tag {tag}"
        )));
    };

    MixFormat {
        sample_rate,
        channels,
        block_align,
        encoding,
    }
    .validate()
    .map_err(Failure::unsupported)
}

fn pcm_encoding(container_bits: u16, valid_bits: u16) -> Result<SampleEncoding, Failure> {
    match (container_bits, valid_bits) {
        (8, 8) => Ok(SampleEncoding::Unsigned8),
        (16 | 24 | 32, valid) if valid > 0 && valid <= container_bits => {
            Ok(SampleEncoding::Signed {
                bytes_per_sample: container_bits / 8,
                valid_bits: valid,
            })
        }
        _ => Err(Failure::unsupported(format_args!(
            "unsupported PCM mix format: {valid_bits}/{container_bits} bits"
        ))),
    }
}
