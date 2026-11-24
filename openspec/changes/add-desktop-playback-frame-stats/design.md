## Context

A native watcher reads decoded frames, schedules them and passes the most recent eligible frame to its UI. These boundaries observe different populations. The sampler belongs to the watch session and survives page changes.

## Goals / Non-Goals

Measure decoded output before scheduling and each unique new frame first accepted by the UI. Keep sampling explicit. Decoder input, compressed payload bitrate and a default background-history policy require separate evidence. Integrating an export must use the diagnostics session facts frozen when the user requests it.

## Decisions

Use a decoder-bound counter for successful reads and validate view, decoder and sequence at UI acceptance. An independent deadline advances at one-second intervals. Rates use actual elapsed monotonic time. Missed deadlines and decoder transitions invalidate the interval. The time horizon and number of samples and markers are bounded. Stop cancels the sampler and a new view starts empty.

Use the existing egui Painter for a solid, fixed-width, draggable card inside the player. Both curves share a timestamp and cursor. Use local marks with no free text. Repaint only while visible as needed. Closing the card can leave sampling active until explicitly stopped.

## Risks / Trade-offs

OS scheduling or system suspend can interrupt the deadline. Record an unavailable interval instead of averaging the interruption. A UI-accepted frame is not a display-present callback. Benchmark the same optimized binary and changing source in three modes before enabling any default history.
