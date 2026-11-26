## 1. Implementation

- [x] 1.1 Implement bounded monotonic session sampling, decoder epochs, deduplication and local marks.
- [x] 1.2 Connect the three native decoder and UI acceptance paths.
- [x] 1.3 Implement movable player card, graph, cursor, availability and player interactions.

## 2. Verification

- [x] 2.1 Exercise controlled timing, gaps, real zero rates and stale callbacks.
- [x] 2.2 Verify card controls at 1040×700 and boundaries at 680×520.
- [x] 2.3 Run platform-target checks and distinguish them from native operation.
- [x] 2.4 Compare off, sampling-only and chart modes with the same optimized macOS viewer and changing source.
- [x] 2.5 Verify the three sampling modes in an Ubuntu desktop VM and retain the opt-in default.
- [x] 2.6 Exercise diagnostic ZIP outcomes across watch stop, failure and restart, stale measurement callbacks and the diagnostic information control alongside the statistics card.
- [ ] 2.7 Verify Windows native playback, hardware-accelerated Linux playback and physical sleep/resume.

Controlled tests cover the clock, interaction, generation isolation and layout. Previous macOS native release and Ubuntu desktop VM runs cover the three sampling modes. The Ubuntu VM used software decoding. Windows target compilation and host tests are recorded separately from a native device run. Sampling starts disabled for each watch. Playback-statistics export remains a separate change.
