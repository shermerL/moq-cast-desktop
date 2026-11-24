## Context

The capture publisher owns encoder creation and reconstruction. Watch tasks open decoders independently from GPU presentation. The diagnostic ZIP writer already owns archive creation.

## Decisions

- Latch the process-local decoder preference once per accepted watch. Reuse the captured policy when the rendition changes. Software-only failure ends that watch and permits a new automatic watch.
- Attribute codec observations to the session and encoder or decoder instance. Distinguish pending data, unavailable data, actual backend, execution class, source kind, resolution and explicitly reported fallback. A GPU-rendered frame alone supplies no hardware decode evidence.
- Keep the current and most recently ended session in process. On explicit export write versioned UTF-8 JSON with build, platform, local session identifier, role, timestamps, status and only allowlisted observations. Exclude source names, stream paths, credentials, IPs, fingerprints, raw errors and media.
- Validate Mac with a real publication and Windows with two watches on the same host, source and optimized build. Both runs selecting software is a valid observation.

## Risks / Trade-offs

The locked publisher may not expose the active encoder through its public capture handle. Require actual instance evidence before reporting a backend. Runtime verification remains per operating system.

## Sources used by this change

| Platform and role | Requested policy and dimensions | Actual backend and execution |
| --- | --- | --- |
| macOS publish | `mac/src/publication.rs` requests H.264 Auto. ScreenCaptureKit enumeration supplies the selected source's dimensions in desktop points. | The locked capture driver emits its opened encoder name after a subscription produces demand. The VideoToolbox name identifies the encoder. The wrapper leaves acceleration selection to macOS and supplies no session acceleration verdict. |
| Windows publish | `windows/src/media.rs` supplies H.264 Auto or hardware-only and the configured source dimensions. | The capture driver reports the opened encoder. The locked Media Foundation encoder enumerates hardware MFTs. |
| Linux publish | `linux/src/publish/session.rs` requests H.264 Auto. X11 enumeration supplies display dimensions. Portal selections can lack dimensions. | The vendored capture driver reports the opened encoder using the same typed fields. |
| macOS / Windows / Linux watch | Each platform's production playback owner samples the process preference at start. Each decoder creation reads the latched value, including catalog replacements. The selected catalog rendition supplies dimensions until the first decoded frame from this instance supplies its size. | The opened `moq_video::decode::Consumer::name()` supplies the implementation. OpenH264 and libvpx establish software execution. The Media Foundation decoder opens with a D3D manager. Its public handle provides no per-session DXVA execution verdict, so the execution field remains unavailable. GPU texture presentation and Linux CPU frame delivery are reported independently of decoder execution. |

The capture handle in MoQ revision `f8215bc` does not expose encoder identity. The capture driver in `rs/moq-video/src/encode/producer.rs` reports the active sink name as the structured `encoder` field after opening it. The desktop subscriber associates the event with the instrumented publication future. Probe events and device labels are excluded from the summary. A scoped decoder fallback with the SDK's `refused` field contributes only the category `hardware_candidates_refused`. On macOS the locked SDK's `decode/sink.rs` opens the decoder inline, so a refusal warning inside the active decoder-open span is attributable. On Windows and Linux the same file opens it via `worker.rs` on a dedicated OS thread. The worker does not carry the calling span or a media-session ID. Even a software backend opened immediately after a warning supplies no causal link. On those builds the backend name is still observable from the returned consumer, while the reason remains absent. A matching span from an expired generation, an encoder span and an unscoped warning cannot set the current watch's reason. Raw refusal strings are excluded. The encoder driver supplies no corresponding per-instance fallback reason.

Each ZIP carries a build-provenance fingerprint and the application version/platform. A byte-for-byte comparison of validation executables uses the separately recorded SHA-256. Session IDs are process-local. The export request takes a snapshot before enqueueing disk work. The media owner stores the current and last finished session. An initial open, a later open, withdrawal and early failure have distinct evidence. The archive contains no encoded samples.

On Windows the publication task owns the session until completion. An explicit StopSharing requests a cooperative stop and joins the task before a restart. Normal stop records `stopped`, external task cancellation records `cancelled`, and a capture error or unwind records `failed`. An already-ready capture result wins over a simultaneous stop. A stale completion cannot retire the next publication.
