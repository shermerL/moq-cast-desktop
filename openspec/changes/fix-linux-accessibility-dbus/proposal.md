## Why

Enabling Linux accessibility on a running desktop requires the application's AccessKit worker to connect to the session bus. The delivered application selected a D-Bus backend that panicked on that worker and left its window inaccessible.

## What Changes

- Align the Linux capture portal's D-Bus client with the runtime available to the accessibility worker.
- Exercise both consumers in a virtual Linux desktop and add a worker regression to the existing Linux test suite.

## Capabilities

### New Capabilities

- `linux-desktop-dbus` covers accessible window discovery and ScreenCast session cleanup when both use the same application dependency graph.

### Modified Capabilities

None.

## Impact

Linux application and vendored capture manifest. The SDK revision, package versions, media protocol and capture authorization workflow stay at their existing baselines.

AI 撰写：gpt-6.2-astra
