## Why

Windows and Linux need a manually selected relay for sharing and watching beyond LAN discovery.

## What Changes

- Add a native manual connection and directory alongside Nearby on both platforms.
- Reuse each platform's existing media pipeline and enforce one active media role.
- Keep connection credentials in memory and honor manual recovery.

## Capabilities

### New Capabilities

- `manual-relay`: Manual relay connection, discovery, playback and publication on Windows and Linux.

### Modified Capabilities

None.

## Impact

Windows and Linux native applications and shared relay implementation. The pinned SDK, server, default startup and LAN identity contract stay compatible.

The three clients share the protocol and random broadcast identity implementation. Publication keeps video running when optional system audio fails. A failed watch releases its media resources and keeps a single recovery action until its owner is released.

AI 撰写：gpt-6.2-astra
AI 修订：gpt-6.2-astra（三端协议、音频与观看恢复范围）
