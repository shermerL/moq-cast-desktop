## ADDED Requirements

### Requirement: Explicit generation-bound frame history
The watcher SHALL start with sampling disabled, use the existing subscription, count successful decoded reads before scheduling and unique UI frame acceptance independently, and retain at most sixty seconds of monotonic observations.

#### Scenario: Active decoder with a stalled UI
- **WHEN** the decoder produces frames throughout an observed interval and the UI accepts none
- **THEN** decoded output has a positive rate and UI acceptance has a measured zero

#### Scenario: Incomplete or disrupted observation
- **WHEN** a full initial interval has not elapsed, the sampler misses a deadline, or the decoder changes
- **THEN** affected rates are unavailable and the chart breaks the line across the boundary

#### Scenario: A watch is replaced
- **WHEN** an old decoder, UI update or stop arrives after the next watch starts
- **THEN** the new session starts with sampling disabled and empty history and marks; late callbacks cannot change its counters or diagnostics

### Requirement: Native movable statistics card
The player SHALL display a movable opaque card using one time axis for decoded output, UI acceptance and local marks, with unavailable metrics identified explicitly.

#### Scenario: Inspecting diagnostics while using statistics
- **WHEN** the viewer opens playback information with the statistics card active
- **THEN** the player retains the session’s requested policy, observed decoder and execution fields alongside the statistics controls

#### Scenario: Inspecting a sampled interval
- **WHEN** the user opens the card and hovers over the chart
- **THEN** both observed rates use the same timestamp and missing rates are distinguishable from zero

#### Scenario: Moving or dismissing the card
- **WHEN** the user drags the card, changes player size or uses Escape
- **THEN** controls remain reachable within the player and the card handles dismissal before fullscreen exit

#### Scenario: Sampling with the card closed
- **WHEN** the card is closed while sampling remains enabled
- **THEN** the session continues to sample independently of player repaints
