## Why

Users comparing a stalled watch session need to know which codec backend actually ran on each endpoint.

## What Changes

- Show requested and observed codec information in local sharing and playback details.
- Apply an automatic or software-only decoder policy to the next new watch.
- Include current and last-ended session summaries in user-initiated diagnostic ZIP exports.

## Capabilities

### New Capabilities

- `media-diagnostics`: observable local publication and playback backend selection and private session summaries.

## Impact

The three native clients, their local diagnostic export and shared playback information are affected. The wire protocol and dependency revisions stay fixed.
