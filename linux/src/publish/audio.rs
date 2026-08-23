//! Linux system-output capture and Opus publication.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, anyhow};
use moq_tokio::moq_net;
use pipewire as pw;
use pw::spa;
use tokio::sync::{Notify, oneshot};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u32 = 2;
const BYTES_PER_SAMPLE: usize = size_of::<f32>();
#[cfg(test)]
const CHUNK_DURATION: Duration = Duration::from_millis(20);
#[cfg(test)]
const CHUNK_FRAMES: usize = SAMPLE_RATE as usize * CHUNK_DURATION.as_millis() as usize / 1_000;
#[cfg(test)]
const CHUNK_BYTES: usize = CHUNK_FRAMES * CHANNELS as usize * BYTES_PER_SAMPLE;
const QUEUE_CAPACITY: usize = 8;
const GAP_TOLERANCE_US: u64 = 100_000;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Capture system output and publish it as an Opus rendition.
pub(super) async fn publish(
    mut broadcast: moq_net::broadcast::Producer,
    catalog: moq_mux::catalog::Producer,
    clock: moq_mux::Clock,
) -> anyhow::Result<()> {
    let capture = Capture::open(clock).await?;
    let queue = capture.queue.clone();

    let input = moq_audio::encode::Input {
        format: moq_audio::Format::F32,
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
    };
    let options = moq_audio::encode::Options::default();
    let mut producer = moq_audio::encode::Producer::new(&mut broadcast, catalog, input, &options)
        .context("open Opus system-audio producer")?;
    let mut timeline = Timeline::default();

    tracing::info!(backend = capture.backend, "system-audio capture started");
    loop {
        let chunk = queue.recv().await?;
        if timeline.discontinuous(&chunk) {
            producer.discontinuity()?;
            producer.reset_epoch();
            tracing::debug!(stage = "audio", "system-audio capture gap reset the epoch");
        }
        let frame = moq_audio::Frame {
            timestamp: moq_net::Timestamp::from_micros(chunk.timestamp_us)?,
            data: chunk.data.into(),
        };
        producer.write(&frame)?;
    }
}

struct Capture {
    backend: &'static str,
    queue: Arc<PcmQueue>,
    quit: pw::channel::Sender<()>,
    handle: Option<JoinHandle<()>>,
}

impl Capture {
    async fn open(clock: moq_mux::Clock) -> anyhow::Result<Self> {
        PendingCapture::pipewire(clock)?.ready().await
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.quit.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct PendingCapture {
    capture: Capture,
    ready: oneshot::Receiver<Result<(), String>>,
}

impl PendingCapture {
    fn pipewire(clock: moq_mux::Clock) -> anyhow::Result<Self> {
        let queue = Arc::new(PcmQueue::new(QUEUE_CAPACITY));
        let (ready_tx, ready) = oneshot::channel();
        let startup = Arc::new(Startup::new(ready_tx));
        let (quit, quit_rx) = pw::channel::channel();
        let thread_queue = queue.clone();
        let thread_startup = startup.clone();
        let handle = std::thread::Builder::new()
            .name("moqcast-pipewire-audio".to_owned())
            .spawn(move || {
                let result =
                    run_pipewire(thread_queue.clone(), thread_startup.clone(), clock, quit_rx);
                finish_capture(result, &thread_queue, &thread_startup);
            })
            .context("start PipeWire system-audio thread")?;
        Ok(Self {
            capture: Capture {
                backend: "pipewire",
                queue,
                quit,
                handle: Some(handle),
            },
            ready,
        })
    }

    async fn ready(self) -> anyhow::Result<Capture> {
        match tokio::time::timeout(STARTUP_TIMEOUT, self.ready).await {
            Ok(Ok(Ok(()))) => Ok(self.capture),
            Ok(Ok(Err(error))) => Err(anyhow!(error)),
            Ok(Err(_)) => Err(anyhow!("audio capture exited during startup")),
            Err(_) => Err(anyhow!("audio capture startup timed out")),
        }
    }
}

struct Startup {
    sender: Mutex<Option<oneshot::Sender<Result<(), String>>>>,
    started: std::sync::atomic::AtomicBool,
}

impl Startup {
    fn new(sender: oneshot::Sender<Result<(), String>>) -> Self {
        Self {
            sender: Mutex::new(Some(sender)),
            started: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn succeed(&self) {
        self.started
            .store(true, std::sync::atomic::Ordering::Release);
        self.send(Ok(()));
    }

    fn fail(&self, error: String) {
        self.send(Err(error));
    }

    fn send(&self, result: Result<(), String>) {
        if let Some(sender) = self.sender.lock().expect("startup mutex poisoned").take() {
            let _ = sender.send(result);
        }
    }
}

fn finish_capture(result: anyhow::Result<()>, queue: &PcmQueue, startup: &Startup) {
    match result {
        Ok(()) if startup.started.load(std::sync::atomic::Ordering::Acquire) => {
            queue.close(Some("system-audio capture ended".to_owned()));
        }
        Ok(()) => startup.fail("system-audio capture ended during startup".to_owned()),
        Err(error) if startup.started.load(std::sync::atomic::Ordering::Acquire) => {
            queue.close(Some(error.to_string()));
        }
        Err(error) => startup.fail(error.to_string()),
    }
}

fn run_pipewire(
    queue: Arc<PcmQueue>,
    startup: Arc<Startup>,
    clock: moq_mux::Clock,
    quit_rx: pw::channel::Receiver<()>,
) -> anyhow::Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).context("create PipeWire main loop")?;
    let context =
        pw::context::ContextRc::new(&mainloop, None).context("create PipeWire context")?;
    let core = context.connect_rc(None).context("connect to PipeWire")?;
    let properties = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Screen",
        *pw::keys::STREAM_CAPTURE_SINK => "true",
    };
    let stream = pw::stream::StreamRc::new(core, "moqcast-system-audio", properties)
        .context("create PipeWire system-audio stream")?;
    let terminal_error = Rc::new(RefCell::new(None::<String>));

    let _listener = stream
        .add_local_listener::<()>()
        .state_changed({
            let startup = startup.clone();
            let terminal_error = terminal_error.clone();
            let mainloop = mainloop.downgrade();
            move |_, _, _, state| match state {
                pw::stream::StreamState::Streaming => startup.succeed(),
                pw::stream::StreamState::Error(error) => {
                    *terminal_error.borrow_mut() = Some(error.to_string());
                    if let Some(mainloop) = mainloop.upgrade() {
                        mainloop.quit();
                    }
                }
                pw::stream::StreamState::Unconnected => {
                    *terminal_error.borrow_mut() =
                        Some("PipeWire system-audio stream disconnected".to_owned());
                    if let Some(mainloop) = mainloop.upgrade() {
                        mainloop.quit();
                    }
                }
                _ => {}
            }
        })
        .process({
            let queue = queue.clone();
            move |stream, _| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let Some(data) = buffer.datas_mut().first_mut() else {
                    return;
                };
                let offset = data.chunk().offset() as usize;
                let size = data.chunk().size() as usize;
                if !valid_chunk(size, data.chunk().flags()) {
                    return;
                }
                let Some(bytes) = data.data() else {
                    return;
                };
                let Some(bytes) = bytes.get(offset..offset.saturating_add(size)) else {
                    return;
                };
                queue.push(clock.micros(), bytes.to_vec());
            }
        })
        .register()
        .context("register PipeWire system-audio listener")?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(SAMPLE_RATE);
    audio_info.set_channels(CHANNELS);
    let object = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .context("serialize PipeWire audio format")?
    .0
    .into_inner();
    let mut params = [spa::pod::Pod::from_bytes(&values)
        .ok_or_else(|| anyhow!("build PipeWire audio format offer"))?];
    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .context("open PipeWire sink monitor")?;

    let _quit = quit_rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.downgrade();
        move |_| {
            if let Some(mainloop) = mainloop.upgrade() {
                mainloop.quit();
            }
        }
    });

    mainloop.run();
    if let Some(error) = terminal_error.borrow_mut().take() {
        return Err(anyhow!(error));
    }
    Ok(())
}

fn valid_chunk(size: usize, flags: spa::buffer::ChunkFlags) -> bool {
    size > 0
        && !flags.contains(spa::buffer::ChunkFlags::CORRUPTED)
        && size.is_multiple_of(CHANNELS as usize * BYTES_PER_SAMPLE)
}

#[derive(Debug)]
struct PcmChunk {
    sequence: u64,
    timestamp_us: u64,
    data: Vec<u8>,
}

struct PcmQueue {
    capacity: usize,
    state: Mutex<QueueState>,
    notify: Notify,
}

#[derive(Default)]
struct QueueState {
    chunks: VecDeque<PcmChunk>,
    next_sequence: u64,
    closed: bool,
    error: Option<String>,
}

impl PcmQueue {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            state: Mutex::new(QueueState::default()),
            notify: Notify::new(),
        }
    }

    fn push(&self, timestamp_us: u64, data: Vec<u8>) {
        let mut state = self.state.lock().expect("PCM queue mutex poisoned");
        if state.closed {
            return;
        }
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.wrapping_add(1);
        if state.chunks.len() == self.capacity {
            state.chunks.pop_front();
        }
        state.chunks.push_back(PcmChunk {
            sequence,
            timestamp_us,
            data,
        });
        drop(state);
        self.notify.notify_one();
    }

    fn close(&self, error: Option<String>) {
        let mut state = self.state.lock().expect("PCM queue mutex poisoned");
        state.closed = true;
        state.error = error;
        drop(state);
        self.notify.notify_waiters();
    }

    async fn recv(&self) -> anyhow::Result<PcmChunk> {
        loop {
            let notified = self.notify.notified();
            {
                let mut state = self.state.lock().expect("PCM queue mutex poisoned");
                if let Some(chunk) = state.chunks.pop_front() {
                    return Ok(chunk);
                }
                if state.closed {
                    return Err(anyhow!(
                        state
                            .error
                            .take()
                            .unwrap_or_else(|| "system-audio capture stopped".to_owned())
                    ));
                }
            }
            notified.await;
        }
    }

    #[cfg(test)]
    fn pop(&self) -> Option<PcmChunk> {
        self.state
            .lock()
            .expect("PCM queue mutex poisoned")
            .chunks
            .pop_front()
    }
}

#[derive(Default)]
struct Timeline {
    previous: Option<(u64, u64, usize)>,
}

impl Timeline {
    fn discontinuous(&mut self, chunk: &PcmChunk) -> bool {
        let frames = chunk.data.len() / (CHANNELS as usize * BYTES_PER_SAMPLE);
        let discontinuous = self
            .previous
            .is_some_and(|(sequence, timestamp_us, frames)| {
                let expected_us = timestamp_us + frames as u64 * 1_000_000 / SAMPLE_RATE as u64;
                chunk.sequence != sequence.wrapping_add(1)
                    || chunk.timestamp_us > expected_us.saturating_add(GAP_TOLERANCE_US)
            });
        self.previous = Some((chunk.sequence, chunk.timestamp_us, frames));
        discontinuous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(sequence: u64, timestamp_us: u64) -> PcmChunk {
        PcmChunk {
            sequence,
            timestamp_us,
            data: vec![0; CHUNK_BYTES],
        }
    }

    #[test]
    fn bounded_queue_discards_the_oldest_pcm() {
        let queue = PcmQueue::new(2);
        queue.push(0, vec![0]);
        queue.push(1, vec![1]);
        queue.push(2, vec![2]);

        assert_eq!(queue.pop().expect("newer chunk retained").sequence, 1);
        assert_eq!(queue.pop().expect("newest chunk retained").sequence, 2);
        assert!(queue.pop().is_none());
    }

    #[test]
    fn contiguous_pcm_keeps_the_current_epoch() {
        let mut timeline = Timeline::default();
        assert!(!timeline.discontinuous(&chunk(4, 1_000_000)));
        assert!(!timeline.discontinuous(&chunk(5, 1_020_000)));
    }

    #[test]
    fn dropped_pcm_resets_the_epoch() {
        let mut timeline = Timeline::default();
        assert!(!timeline.discontinuous(&chunk(4, 1_000_000)));
        assert!(timeline.discontinuous(&chunk(6, 1_040_000)));
    }

    #[test]
    fn capture_gap_resets_the_epoch() {
        let mut timeline = Timeline::default();
        assert!(!timeline.discontinuous(&chunk(4, 1_000_000)));
        assert!(timeline.discontinuous(&chunk(5, 1_200_001)));
    }

    #[test]
    fn corrupted_or_misaligned_pipewire_chunks_are_discarded() {
        assert!(valid_chunk(CHUNK_BYTES, spa::buffer::ChunkFlags::empty()));
        assert!(!valid_chunk(
            CHUNK_BYTES,
            spa::buffer::ChunkFlags::CORRUPTED
        ));
        assert!(!valid_chunk(0, spa::buffer::ChunkFlags::empty()));
        assert!(!valid_chunk(3, spa::buffer::ChunkFlags::empty()));
    }
}
