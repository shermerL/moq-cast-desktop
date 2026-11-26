## 1. Implementation

- [x] 1.1 Align the vendored portal D-Bus backend with AccessKit and check locked SDK and package versions.
- [x] 1.2 Add a Linux regression using the accessibility worker's executor and an unavailable Unix socket.

## 2. Virtual acceptance

- [x] 2.1 Reproduce the missing accessibility tree with the old package, then query native controls and verify normal exit with the candidate.
- [x] 2.2 Exercise ScreenCast creation and cleanup from the candidate, including an unsuccessful negotiation, and record which services and outputs were virtual.
- [x] 2.3 Run the worker and existing portal regressions and validate the change.

AI 撰写：gpt-6.2-astra
