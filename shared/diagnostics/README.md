# Shared diagnostics

The three desktop applications use this crate for local logging, the log viewer,
and ZIP export. Logging never performs disk I/O or waits for queue capacity on
network or media producer threads.

## Retention

- `moqcast.log` and its numbered rotations contain INFO, WARN and ERROR events,
  including application connection/capture/playback lifecycle and media summaries.
  The default budget remains five files of up to 8 MiB each.
- `moqcast-detail.log` and its numbered rotation contain enabled DEBUG and TRACE
  events. They have a separate short window of two files of up to 8 MiB each.
- Both streams use the existing single writer thread and one 4,096-command FIFO.
  At most 2,048 queued detail events can occupy it. The remaining capacity is
  reserved from detail traffic; lifecycle events may use the whole queue.
- The in-app viewer retains up to 1,000 lifecycle and 1,000 detailed entries in
  independent rings, presented in insertion order. Detail events cannot evict
  lifecycle entries or hold their insertion lock.

These are bounded windows, not an audit journal. Lifecycle overload or lock
contention can still drop events. Filesystem failures, including a full disk,
are not solved by queue reservations. The combined default disk budget is
56 MiB, excluding user-created ZIPs and existing unrelated files. Existing mixed
logs are retained as history; the split applies to newly emitted events.

## Export and failures

Export is a FIFO barrier on the same worker: accepted lines before the barrier
are written and both streams flushed before copying current and rotated files.
Shutdown likewise drains preceding commands and flushes both streams. Disabling
detailed mode filters future detail events without clearing retained evidence.

`environment.txt` preserves `dropped_diagnostics` and adds:

- `writer_queue_dropped`: line commands rejected because the queue was full or closed;
- `detail_queue_dropped` and `lifecycle_queue_dropped`: the queue count by stream;
- `ring_contention_dropped`: events rejected by a busy in-memory ring;
- `output_errors=0`: only present in a successful export. Any observed write or
  flush error fails export with an I/O error rather than producing a healthy archive.

Counters are process-local observational snapshots taken when requesting export.
A ring rejection does not prove a disk-log record was lost, and a record rejected
in both places contributes to both counters. None counts network packet loss or
normal retention-window eviction.

Both streams share the existing private-file policy, age retention, and export
path/hard-link protection. Each rotating writer only prunes its own stream, so it
cannot unlink the other writer's active file.

## Validation

Run `cargo test --locked --manifest-path shared/diagnostics/Cargo.toml` from the
repository root. Existing Windows, macOS and Linux workflows run this suite.
Tests cover trace-driven rotation, bounded memory, queue pressure, export and
shutdown ordering, runtime detail toggles, output errors, and export protection.
