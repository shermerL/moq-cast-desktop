# MoQCast Desktop

Native screen sharing and playback for Windows, macOS, and Linux, built on MoQ.

## Context

- Read `CONTRIBUTING.md`, the affected platform's README, and any nested `AGENTS.md` before changing that area.
- Use `.github/workflows/` for current target-specific toolchains and validation commands.
- Keep this file focused on durable conventions. Put platform details in the relevant documentation.
- Change agent instructions only when requested; keep them concise and explain goals rather than prescribing every step.

## Development Rules

- Fix the root cause. Do not hide a fixable defect with arbitrary retries, sleeps, or longer timeouts.
- Reject unsupported or malformed input with an actionable error. Do not silently continue with an invalid media or connection state.
- Keep changes focused on one independently reviewable concern. Avoid unrelated refactors and formatting churn; update affected docs with the change.
- Make resource ownership and shutdown responsibilities explicit. Keep temporary state local and internal APIs private until a consumer needs them.
- Prefer existing conventions and small, clear abstractions. Comments explain non-obvious reasons, not the history of a change.
- Reproduce defects when feasible and add a regression test when practical. Use controlled time for timing tests and wire new tests into CI.
- Explain public API, protocol, and interoperability impacts. Changes to upstream MoQ require separate authorization.
- Preserve other work in progress. One writer owns a shared checkout at a time; coordinate before changing branches, the index, or files.
- Follow the maintainer's current Git handoff and authorization. Confirm the exact complete commit title before committing; implementation approval alone does not authorize pushing, merging, publishing, or upstream PRs.
- Use feature-based branch names without agent or model names. Do not bump release versions without a request.

## Cross-Platform Validation

- Application `main` uses a fixed upstream MoQ `main` baseline. Upstream `dev` dependencies belong on application `dev` or an approved topic branch. Main-based work targets application `main`, without automatic propagation to `dev`.
- Audit dependency changes across manifests, lockfiles, target features, vendor patches, and build metadata. Remove a local patch only after confirming an equivalent upstream implementation.
- Match checks to the affected OS, architecture, features, and system libraries. A host-only check does not validate another platform.
- Full compilation, tests, Clippy, and packaging default to GitHub CI. Perform necessary focused local checks and report checks not run.
- Keep source inspection, local checks, CI results, packaged artifacts, and real-device validation distinct. CI success does not establish device acceptance.
- Measure performance-sensitive changes under representative load instead of relying on intuition.

| Change | Also check |
| --- | --- |
| Shared crates or UI | Platform consumers and workflow path filters |
| MoQ dependency or vendor | Affected platform manifests/locks, retained patches, and interoperability |
| Build or packaging | Target features, scripts, workflow inputs, and packaged source identifiers |
| Discovery or session lifecycle | Stop/restart, cancellation, reconnection, and stale callbacks |

## Code Review Rules

Report actionable defects introduced by the change, with the triggering scenario,
affected platform, and concrete failure path. Separate confirmed findings from
hypotheses. Keep formatting and lint enforcement in CI; avoid unrelated cleanup findings.

- Check resource release on early errors, cancellation, stop, replacement, and shutdown. Stale tasks must not update or close a newer session, and existing cleanup completion guarantees must remain intact.
- Check cancellation safety of futures dropped by `select!`. Decoder polling must not accidentally discard compressed frames; intentional freshness policies are a separate decision.
- Volume and mute control only the watched stream. Muting preserves audio-track processing and its playback clock. Freshness budgets are not guarantees of end-to-end latency or audio/video synchronization.
- MoQ content identities must stay consistent across publication restarts. Review name reuse and sequence resets against the selected upstream contract without conflating stream identity with device identity.
- Check dependency and cross-platform delivery requirements above. Diagnostic and packaged source identifiers must describe the code actually built.
- Discovery is not authentication. Preserve certificate and credential validation; friendly device names are not trusted identities. Keep credentials and private connection details out of routine logs and public comments.
