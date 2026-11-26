## ADDED Requirements

### Requirement: Manual relay and trust

The native application SHALL accept a manual moqt or https URL and optional certificate fingerprint. A fingerprint SHALL be scoped to scheme, host and effective port. New endpoints require an explicit subsequent connection after clearing a stale pin.

#### Scenario: Manual reconnect

- **WHEN** a user cancels, disconnects or loses a connection
- **THEN** the application releases that connection and its own media and waits for a manual reconnect without republishing or rewatching.

### Requirement: Native media roles

Windows and Linux SHALL support manually watching an announced revision and publishing a supported native capture source under a new broadcast identity on each start, using their existing audio capabilities.

#### Scenario: Publication is stopped and restarted

- **WHEN** the user stops sharing and starts again with the same prefix
- **THEN** the old route is withdrawn and the new route has a fresh identity and requires manual selection.

#### Scenario: Scoped publication

- **WHEN** an announcement is locally active without a per-broadcast server receipt
- **THEN** the UI reports local sharing and allows stopping, without inferring rejection or remote visibility.

### Requirement: Media isolation

The application SHALL enforce one media role, bind operations to the selected connection and route, and preserve the independent transport when Nearby or Relay is shut down.

#### Scenario: Late result across sessions

- **WHEN** a stale route, media result or disconnect arrives after a new session has started
- **THEN** it cannot change or stop the new session.

#### Scenario: Nearby disabled during relay media

- **WHEN** Nearby is switched off during relay publishing or watching
- **THEN** the relay connection, the active media and its diagnostics remain owned until that media ends.

### Requirement: Common broadcast identity

The three applications SHALL use one random UUID v4 generation path for every publication and independent publish/receive origins. Connection summaries SHALL exclude private credentials and scope.

#### Scenario: New process reuses a prefix

- **WHEN** a restarted publisher publishes again with the same prefix
- **THEN** its broadcast identity differs, and an old epoch or stop cannot withdraw or select the new instance.

### Requirement: Optional system audio

The publisher SHALL keep video on the same publication when optional audio fails. Windows Relay SHALL start with audio off and capture the user’s choice in the request. Nearby SHALL retain its existing audio behavior.

#### Scenario: Disabled, silent and failed audio

- **WHEN** Windows Relay starts with audio disabled
- **THEN** no WASAPI device opens and no audio rendition is advertised.
- **WHEN** the user enables Windows system audio
- **THEN** the UI explains that the default output includes sounds from other windows and the existing capture path is used.
- **WHEN** Linux audio cannot start or terminates unexpectedly
- **THEN** the audio owner is released, the failure is reported and the same video publication continues until its own end or explicit stop.
- **WHEN** a functioning audio device supplies silence
- **THEN** silence alone does not mark the backend failed.

### Requirement: Terminal watch recovery

An active watch SHALL release media resources and clear the last frame after withdrawal or stream failure while retaining its logical owner, generation and source until acknowledged.

#### Scenario: A second announcement appears during recovery

- **WHEN** a watch has failed and a new announcement becomes available
- **THEN** a single stop-and-return action releases that watch before another media task can start. An enqueue failure leaves the action available to retry.

#### Scenario: The connection is released

- **WHEN** an explicit disconnect or connection closure has already released the watch owner
- **THEN** the player offers return to the source without a second stop, and Nearby media on a separate transport remains active.

AI 撰写：gpt-6.2-astra
AI 修订：gpt-6.2-astra（新增身份、声音和终态契约）
