## Context

The macOS implementation already connects a publisher origin and an independent receiver origin to one relay. Windows and Linux own native capture, audio, playback and diagnostics in their runtime supervisors.

## Goals / Non-Goals

Support a single manually connected relay independently of Nearby. Keep the native media owner, capture choices, audio behavior, diagnostics and default-off statistics for each session. Automatic reconnection and multi-relay forwarding are outside this change.

## Decisions

Share the verified connection, certificate, route revision and withdrawal behavior. Route a selected broadcast to the existing native decoder, or publish the selected native source on a fresh relative path. Give commands and callbacks a connection and media identity; release a pending request before shutting down its origin. Turning off one transport stops only that transport's media.

## Risks / Trade-offs

A server can silently filter announcements. Local publication and independent remote decode are separate observations. Test synthetic sources under isolated Linux automation, distinguish Windows target compilation from Windows execution, and retain device, audible monitoring and cross-network checks for integration.

## Publication and watch ownership

All three clients generate a random UUID v4 for each publication, including after a process restart. Publish and receive origins are independent. The connection owner applies exact announcement epochs, scopes certificate trust by endpoint, redacts credentials and waits for manual reconnection.

A Linux audio startup or runtime failure releases the PipeWire stream and Opus producer and reports the failure in the current publication. A silent but functioning source remains usable. Keep the same in-flight video future until video ends or the user stops. Drop capture futures before awaiting the portal cleanup owner; surface cleanup failures. Windows Relay snapshots the sound choice with the publish command, initially off. Off skips WASAPI and creates no audio track. On uses the computer’s default output, including audio from other windows. Nearby keeps its original sound behavior.

When an announcement disappears or an active stream fails, stop and join playback, clear frames and retain the watch generation, route and logical media slot. The sole recovery action stops that generation and returns to its source. Explicit stop, disconnect or a closed owning connection releases the slot. Windows Nearby stop commands carry the media generation as well. Shutting down one transport preserves media on the other.

## Device acceptance

Run three native rounds, one publisher and two concurrent receivers per round. Once the stable path is covered, run cross-network loss and manual reconnection separately. Verify Mac main-display audio at a physical receiving machine and capture normal-size and fullscreen windows during the rounds. Observe Wayland restore tokens, portal indicators and Windows recovery from real silence. Only a reproducible user-visible problem warrants a follow-up. Do not introduce a synthetic subscriber or change timestamp or sink behavior to anticipate one.

AI 撰写：gpt-6.2-astra
AI 修订：gpt-6.2-astra（两端运行时、平台声音策略与实机验收次序）
