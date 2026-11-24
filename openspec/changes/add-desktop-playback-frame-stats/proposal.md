## Why

Playback troubleshooting needs independent observations of decoder output and UI delivery. A single rate sampled by an open popup cannot distinguish media stalls from a slow consumer.

## What Changes

- Add an explicitly enabled, session-local 60-second frame history to native watchers.
- Display two observed rates and timestamped local markers in a movable player card.
- Keep payload bitrate and decoder input unavailable until the existing media subscription exposes these measurements.

## Capabilities

### New Capabilities

- `playback-frame-stats`: opt-in native frame sampling and chart interaction.

### Modified Capabilities

None.

## Impact

Shared Rust sampling and egui UI, and the macOS, Windows and Linux native viewing paths. The media backend, network protocol and subscription count stay the same.
