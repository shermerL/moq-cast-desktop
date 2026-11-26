## ADDED Requirements

### Requirement: The application exposes its window to assistive technology

The Linux application SHALL serve its accessible controls on AT-SPI when a screen reader is enabled, using the same build that supports portal capture.

#### Scenario: Inspect the Relay form

- **WHEN** the user opens the Relay form with assistive technology enabled
- **THEN** the assistive client can read the name and role of a form control
- **AND** closing the application removes its entry from the accessibility desktop

#### Scenario: The accessibility worker cannot reach the bus

- **WHEN** the worker attempts to connect to a missing Unix socket
- **THEN** the worker receives an I/O error and returns normally

### Requirement: ScreenCast sessions retain their cleanup owner

The application SHALL release each created ScreenCast session through its existing capture owner when capture stops or a request fails after creation.

#### Scenario: Release a created session

- **WHEN** the user stops a granted capture or negotiation ends after the session was created
- **THEN** the application requests closure of that session and observes the result before completing publication teardown

AI 撰写：gpt-6.2-astra
