//! Windows system-output capture, PCM normalization, and Opus publication.

#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use thiserror::Error;

pub(crate) const OUTPUT_SAMPLE_RATE: u32 = 48_000;
pub(crate) const OUTPUT_CHANNELS: u32 = 2;

#[cfg(target_os = "windows")]
mod wasapi;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AudioPhase {
    #[default]
    Idle,
    Preparing,
    Publishing,
    Silent,
    Recovering,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioIssue {
    NoDefaultOutput,
    DefaultOutputChanged,
    DeviceInvalidated,
    UnsupportedMixFormat,
    CaptureBackend,
    Encoder,
}

impl AudioIssue {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::NoDefaultOutput => "No default Windows output device is available.",
            Self::DefaultOutputChanged => {
                "The default Windows output device changed; reconnecting."
            }
            Self::DeviceInvalidated => "The Windows output device was invalidated; reconnecting.",
            Self::UnsupportedMixFormat => {
                "The default output device uses an unsupported mix format."
            }
            Self::CaptureBackend => "Windows system-audio capture is unavailable.",
            Self::Encoder => "System audio could not be encoded as Opus.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioEvent {
    Started {
        input_sample_rate: u32,
        input_channels: u16,
    },
    Active,
    Silent,
    Recovering(AudioIssue),
    Failed(AudioIssue),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatusUpdate {
    pub(crate) generation: u64,
    pub(crate) event: AudioEvent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AudioSnapshot {
    pub(crate) generation: u64,
    pub(crate) phase: AudioPhase,
    pub(crate) input_sample_rate: Option<u32>,
    pub(crate) input_channels: Option<u16>,
    pub(crate) output_sample_rate: u32,
    pub(crate) output_channels: u32,
    pub(crate) last_error: Option<&'static str>,
}

impl Default for AudioSnapshot {
    fn default() -> Self {
        Self {
            generation: 0,
            phase: AudioPhase::Idle,
            input_sample_rate: None,
            input_channels: None,
            output_sample_rate: OUTPUT_SAMPLE_RATE,
            output_channels: OUTPUT_CHANNELS,
            last_error: None,
        }
    }
}

impl AudioSnapshot {
    pub(crate) fn begin(&mut self, generation: u64) {
        self.generation = generation;
        self.phase = AudioPhase::Preparing;
        self.input_sample_rate = None;
        self.input_channels = None;
        self.last_error = None;
    }

    pub(crate) fn apply(&mut self, update: StatusUpdate) -> bool {
        if update.generation != self.generation
            || matches!(self.phase, AudioPhase::Idle | AudioPhase::Stopping)
        {
            return false;
        }

        match update.event {
            AudioEvent::Started {
                input_sample_rate,
                input_channels,
            } => {
                self.phase = AudioPhase::Publishing;
                self.input_sample_rate = Some(input_sample_rate);
                self.input_channels = Some(input_channels);
                self.last_error = None;
            }
            AudioEvent::Active => {
                self.phase = AudioPhase::Publishing;
                self.last_error = None;
            }
            AudioEvent::Silent => {
                self.phase = AudioPhase::Silent;
                self.last_error = None;
            }
            AudioEvent::Recovering(issue) => {
                self.phase = AudioPhase::Recovering;
                self.last_error = Some(issue.message());
            }
            AudioEvent::Failed(issue) => {
                self.phase = AudioPhase::Failed;
                self.last_error = Some(issue.message());
            }
        }
        true
    }

    pub(crate) fn begin_stop(&mut self, generation: u64) {
        if generation == self.generation && self.phase != AudioPhase::Idle {
            self.phase = AudioPhase::Stopping;
        }
    }

    pub(crate) fn ended(&mut self, generation: u64) {
        if generation != self.generation {
            return;
        }
        self.phase = AudioPhase::Idle;
        self.input_sample_rate = None;
        self.input_channels = None;
        self.last_error = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SampleEncoding {
    Float32,
    Unsigned8,
    Signed {
        bytes_per_sample: u16,
        valid_bits: u16,
    },
}

impl SampleEncoding {
    fn bytes_per_sample(self) -> usize {
        match self {
            Self::Float32 => 4,
            Self::Unsigned8 => 1,
            Self::Signed {
                bytes_per_sample, ..
            } => usize::from(bytes_per_sample),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MixFormat {
    pub(super) sample_rate: u32,
    pub(super) channels: u16,
    pub(super) block_align: u16,
    pub(super) encoding: SampleEncoding,
}

#[derive(Debug, Error, PartialEq, Eq)]
enum PcmError {
    #[error("audio format contains no channels or sample rate")]
    EmptyFormat,
    #[error("audio block alignment is smaller than its samples")]
    InvalidBlockAlignment,
    #[error("audio packet length does not match its declared frame count")]
    TruncatedPacket,
    #[error("audio packet is not aligned to its channel count")]
    MisalignedChannels,
    #[error("system audio currently supports only mono or stereo mix formats")]
    UnsupportedChannels,
    #[error("unsupported PCM container")]
    UnsupportedContainer,
}

impl MixFormat {
    fn validate(self) -> Result<Self, PcmError> {
        if self.sample_rate == 0 || self.channels == 0 {
            return Err(PcmError::EmptyFormat);
        }
        if self.channels > 2 {
            return Err(PcmError::UnsupportedChannels);
        }
        let sample_bytes = self.encoding.bytes_per_sample();
        if usize::from(self.block_align) < sample_bytes * usize::from(self.channels) {
            return Err(PcmError::InvalidBlockAlignment);
        }
        Ok(self)
    }

    fn decode(self, packet: &[u8], frames: u32) -> Result<Vec<f32>, PcmError> {
        let format = self.validate()?;
        let frame_bytes = usize::from(format.block_align);
        let frame_count = usize::try_from(frames).map_err(|_| PcmError::TruncatedPacket)?;
        let expected = frame_count
            .checked_mul(frame_bytes)
            .ok_or(PcmError::TruncatedPacket)?;
        if packet.len() < expected {
            return Err(PcmError::TruncatedPacket);
        }

        let channels = usize::from(format.channels);
        let sample_bytes = format.encoding.bytes_per_sample();
        let mut samples = Vec::with_capacity(frame_count * channels);
        for frame in packet[..expected].chunks_exact(frame_bytes) {
            for channel in 0..channels {
                let offset = channel * sample_bytes;
                samples.push(decode_sample(
                    format.encoding,
                    &frame[offset..offset + sample_bytes],
                )?);
            }
        }
        Ok(samples)
    }
}

fn decode_sample(encoding: SampleEncoding, bytes: &[u8]) -> Result<f32, PcmError> {
    match encoding {
        SampleEncoding::Float32 if bytes.len() == 4 => Ok(f32::from_le_bytes(
            bytes.try_into().expect("four-byte float sample"),
        )
        .clamp(-1.0, 1.0)),
        SampleEncoding::Unsigned8 if bytes.len() == 1 => Ok((f32::from(bytes[0]) - 128.0) / 128.0),
        SampleEncoding::Signed {
            bytes_per_sample,
            valid_bits,
        } if usize::from(bytes_per_sample) == bytes.len()
            && matches!(bytes_per_sample, 2..=4)
            && valid_bits > 0
            && valid_bits <= bytes_per_sample * 8 =>
        {
            let signed = match bytes_per_sample {
                2 => i32::from(i16::from_le_bytes(
                    bytes.try_into().expect("two-byte PCM sample"),
                )),
                3 => {
                    let raw = i32::from(bytes[0])
                        | (i32::from(bytes[1]) << 8)
                        | (i32::from(bytes[2]) << 16);
                    (raw << 8) >> 8
                }
                4 => i32::from_le_bytes(bytes.try_into().expect("four-byte PCM sample")),
                _ => unreachable!("guarded PCM width"),
            };
            let container_bits = u32::from(bytes_per_sample) * 8;
            let shift = container_bits - u32::from(valid_bits);
            let value = signed >> shift;
            let scale = (1_u64 << (u32::from(valid_bits) - 1)) as f32;
            Ok((value as f32 / scale).clamp(-1.0, 1.0))
        }
        _ => Err(PcmError::UnsupportedContainer),
    }
}

fn remix_to_stereo(samples: &[f32], channels: u16) -> Result<Vec<f32>, PcmError> {
    let channels = usize::from(channels);
    if channels == 0 || !samples.len().is_multiple_of(channels) {
        return Err(PcmError::MisalignedChannels);
    }
    match channels {
        1 => {
            let mut output = Vec::with_capacity(samples.len() * 2);
            for &sample in samples {
                output.extend_from_slice(&[sample, sample]);
            }
            Ok(output)
        }
        2 => Ok(samples.to_vec()),
        _ => Err(PcmError::UnsupportedChannels),
    }
}

#[cfg(target_os = "windows")]
struct Normalizer {
    format: MixFormat,
    resampler: Option<moq_audio::Resampler>,
}

#[cfg(target_os = "windows")]
impl Normalizer {
    fn new(format: MixFormat) -> anyhow::Result<Self> {
        let format = format.validate()?;
        let chunk_frames = usize::try_from((format.sample_rate / 100).max(1))?;
        let resampler = (format.sample_rate != OUTPUT_SAMPLE_RATE)
            .then(|| {
                moq_audio::Resampler::new(
                    format.sample_rate,
                    OUTPUT_SAMPLE_RATE,
                    OUTPUT_CHANNELS,
                    chunk_frames,
                )
            })
            .transpose()?;
        Ok(Self { format, resampler })
    }

    fn normalize(&mut self, samples: &[f32]) -> anyhow::Result<Vec<f32>> {
        let stereo = remix_to_stereo(samples, self.format.channels)?;
        match self.resampler.as_mut() {
            Some(resampler) => Ok(resampler.process(&stereo)?),
            None => Ok(stereo),
        }
    }

    fn reset(&mut self) -> anyhow::Result<()> {
        *self = Self::new(self.format)?;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
pub(crate) async fn publish(
    mut broadcast: moq_tokio::moq_net::broadcast::Producer,
    catalog: moq_mux::catalog::Producer,
    clock: moq_mux::Clock,
    generation: u64,
    updates: tokio::sync::watch::Sender<Option<StatusUpdate>>,
) {
    use std::time::Duration;

    let mut options = moq_audio::encode::Options::default();
    options.track = Some("0.opus".to_owned());
    options.codec = moq_audio::encode::Codec::Opus;
    options.sample_rate = Some(OUTPUT_SAMPLE_RATE);
    options.channels = Some(OUTPUT_CHANNELS);
    options.dtx = true;

    let input = moq_audio::encode::Input {
        format: moq_audio::Format::F32,
        sample_rate: OUTPUT_SAMPLE_RATE,
        channels: OUTPUT_CHANNELS,
    };
    let mut producer =
        match moq_audio::encode::Producer::new(&mut broadcast, catalog, input, &options) {
            Ok(producer) => producer,
            Err(error) => {
                tracing::warn!(stage = "audio-encode", %error, "could not create Opus audio track");
                emit(
                    &updates,
                    generation,
                    AudioEvent::Failed(AudioIssue::Encoder),
                );
                return;
            }
        };

    let mut restarting = false;
    loop {
        let mut capture = match wasapi::Capture::start(clock) {
            Ok(capture) => capture,
            Err(error) => {
                tracing::warn!(stage = "audio-capture-thread", %error, "could not start WASAPI loopback thread");
                emit(
                    &updates,
                    generation,
                    AudioEvent::Failed(AudioIssue::CaptureBackend),
                );
                return;
            }
        };
        let mut normalizer = None;
        let mut gap = restarting;
        let mut reported_silent = false;

        loop {
            match tokio::time::timeout(Duration::from_millis(750), capture.recv()).await {
                Ok(Some(wasapi::CaptureEvent::Ready(format))) => {
                    normalizer = match Normalizer::new(format) {
                        Ok(normalizer) => Some(normalizer),
                        Err(error) => {
                            tracing::warn!(stage = "audio-normalize", %error, "unsupported WASAPI mix format");
                            emit(
                                &updates,
                                generation,
                                AudioEvent::Failed(AudioIssue::UnsupportedMixFormat),
                            );
                            return;
                        }
                    };
                    emit(
                        &updates,
                        generation,
                        AudioEvent::Started {
                            input_sample_rate: format.sample_rate,
                            input_channels: format.channels,
                        },
                    );
                }
                Ok(Some(wasapi::CaptureEvent::Packet(packet))) => {
                    let Some(normalizer) = normalizer.as_mut() else {
                        continue;
                    };
                    if gap || packet.discontinuity {
                        if let Err(error) = producer.discontinuity() {
                            tracing::warn!(stage = "audio-publish", %error, "could not mark audio discontinuity");
                            emit(
                                &updates,
                                generation,
                                AudioEvent::Failed(AudioIssue::Encoder),
                            );
                            return;
                        }
                        producer.reset_epoch();
                        if let Err(error) = normalizer.reset() {
                            tracing::warn!(stage = "audio-normalize", %error, "could not reset audio resampler");
                            emit(
                                &updates,
                                generation,
                                AudioEvent::Failed(AudioIssue::Encoder),
                            );
                            return;
                        }
                        gap = false;
                    }

                    let samples = match normalizer.normalize(&packet.samples) {
                        Ok(samples) => samples,
                        Err(error) => {
                            tracing::warn!(stage = "audio-normalize", %error, "could not normalize system audio");
                            emit(
                                &updates,
                                generation,
                                AudioEvent::Failed(AudioIssue::Encoder),
                            );
                            return;
                        }
                    };
                    if !samples.is_empty() {
                        let mut data =
                            Vec::with_capacity(samples.len() * std::mem::size_of::<f32>());
                        for sample in samples {
                            data.extend_from_slice(&sample.to_le_bytes());
                        }
                        let timestamp = match moq_tokio::moq_net::Timestamp::from_micros(
                            packet.timestamp_us,
                        ) {
                            Ok(timestamp) => timestamp,
                            Err(error) => {
                                tracing::warn!(stage = "audio-clock", %error, "invalid audio timestamp");
                                emit(
                                    &updates,
                                    generation,
                                    AudioEvent::Failed(AudioIssue::Encoder),
                                );
                                return;
                            }
                        };
                        if let Err(error) = producer.write(&moq_audio::Frame {
                            timestamp,
                            data: data.into(),
                        }) {
                            tracing::warn!(stage = "audio-publish", %error, "could not publish Opus audio");
                            emit(
                                &updates,
                                generation,
                                AudioEvent::Failed(AudioIssue::Encoder),
                            );
                            return;
                        }
                    }

                    if packet.silent {
                        if !reported_silent {
                            emit(&updates, generation, AudioEvent::Silent);
                        }
                        reported_silent = true;
                    } else {
                        if reported_silent {
                            emit(&updates, generation, AudioEvent::Active);
                        }
                        reported_silent = false;
                    }
                }
                Ok(Some(wasapi::CaptureEvent::Stopped(failure))) => {
                    tracing::warn!(
                        stage = "audio-capture",
                        reason = ?failure.kind,
                        detail = %failure.detail,
                        "WASAPI loopback stopped"
                    );
                    if let Some(issue) = failure.kind.recovery_issue() {
                        emit(&updates, generation, AudioEvent::Recovering(issue));
                        break;
                    }
                    emit(
                        &updates,
                        generation,
                        AudioEvent::Failed(failure.kind.issue()),
                    );
                    return;
                }
                Ok(None) => {
                    emit(
                        &updates,
                        generation,
                        AudioEvent::Recovering(AudioIssue::CaptureBackend),
                    );
                    break;
                }
                Err(_) => {
                    if normalizer.is_some() && !reported_silent {
                        emit(&updates, generation, AudioEvent::Silent);
                    }
                    reported_silent = true;
                    gap = true;
                }
            }
        }

        drop(capture);
        restarting = true;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(target_os = "windows")]
fn emit(
    updates: &tokio::sync::watch::Sender<Option<StatusUpdate>>,
    generation: u64,
    event: AudioEvent,
) {
    let _ = updates.send(Some(StatusUpdate { generation, event }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_audio_generation_cannot_override_current_publication() {
        let mut audio = AudioSnapshot::default();
        audio.begin(2);
        assert!(!audio.apply(StatusUpdate {
            generation: 1,
            event: AudioEvent::Failed(AudioIssue::CaptureBackend),
        }));
        assert_eq!(audio.phase, AudioPhase::Preparing);
    }

    #[test]
    fn audio_failure_is_typed_and_can_recover_without_media_state() {
        let mut audio = AudioSnapshot::default();
        audio.begin(7);
        assert!(audio.apply(StatusUpdate {
            generation: 7,
            event: AudioEvent::Recovering(AudioIssue::DeviceInvalidated),
        }));
        assert_eq!(audio.phase, AudioPhase::Recovering);
        assert!(audio.apply(StatusUpdate {
            generation: 7,
            event: AudioEvent::Started {
                input_sample_rate: 44_100,
                input_channels: 2,
            },
        }));
        assert_eq!(audio.phase, AudioPhase::Publishing);
        assert_eq!(audio.input_sample_rate, Some(44_100));
        assert_eq!(audio.output_sample_rate, 48_000);
        assert_eq!(audio.output_channels, 2);
    }

    #[test]
    fn stopping_rejects_late_audio_callbacks() {
        let mut audio = AudioSnapshot::default();
        audio.begin(4);
        audio.begin_stop(4);
        assert!(!audio.apply(StatusUpdate {
            generation: 4,
            event: AudioEvent::Active,
        }));
        assert_eq!(audio.phase, AudioPhase::Stopping);
        audio.ended(4);
        assert_eq!(audio.phase, AudioPhase::Idle);
    }

    #[test]
    fn pcm_mix_format_decodes_float_and_padded_frames() {
        let format = MixFormat {
            sample_rate: 48_000,
            channels: 2,
            block_align: 12,
            encoding: SampleEncoding::Float32,
        };
        let mut packet = Vec::new();
        packet.extend_from_slice(&0.25_f32.to_le_bytes());
        packet.extend_from_slice(&(-0.5_f32).to_le_bytes());
        packet.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(format.decode(&packet, 1).unwrap(), [0.25, -0.5]);
    }

    #[test]
    fn pcm_mix_format_decodes_signed_24_bit() {
        let format = MixFormat {
            sample_rate: 48_000,
            channels: 1,
            block_align: 3,
            encoding: SampleEncoding::Signed {
                bytes_per_sample: 3,
                valid_bits: 24,
            },
        };
        let samples = format.decode(&[0, 0, 64, 0, 0, 192], 2).unwrap();
        assert!((samples[0] - 0.5).abs() < 0.0001);
        assert!((samples[1] + 0.5).abs() < 0.0001);
    }

    #[test]
    fn mono_pcm_is_duplicated_and_unknown_multichannel_is_rejected() {
        assert_eq!(
            remix_to_stereo(&[0.25, -0.5], 1).unwrap(),
            [0.25, 0.25, -0.5, -0.5]
        );
        assert_eq!(
            remix_to_stereo(&[0.2, -0.2, 0.2, 0.4, 0.1, -0.1], 6),
            Err(PcmError::UnsupportedChannels)
        );
    }

    #[test]
    fn silent_unsigned_pcm_maps_to_zero() {
        let format = MixFormat {
            sample_rate: 8_000,
            channels: 1,
            block_align: 1,
            encoding: SampleEncoding::Unsigned8,
        };
        assert_eq!(format.decode(&[128], 1).unwrap(), [0.0]);
    }
}
