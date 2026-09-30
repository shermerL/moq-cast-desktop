# Contributing to MoQCast Desktop

Thank you for helping improve MoQCast Desktop. Keep each change focused on one independently reviewable and reversible concern.

## Branches

- `main` is the stable and release line.
- `main` uses a fixed baseline that has landed on upstream `moq-dev/moq` main.
- `dev` is reserved for work based on upstream MoQ dev.
- Create main-based features and fixes from `origin/main` and integrate them into `main`; do not automatically synchronize them into `dev`.
- Create upstream-dev-based work from `origin/dev`. Dependency upgrades and branch promotion require the maintainer's authorization; record any retained fork or vendor patches.

Topic branches use this format:

```text
<base>-<scope>/<topic>
```

`<base>` must be `main` or `dev`. Common scopes include `desktop`, `windows`, `linux`, `macos`, and `windows-lite`.

Examples:

```text
dev-windows/audio-recovery
dev-desktop/diagnostic-logs
main-windows/release-hotfix
```

Do not use `main/...` or `dev/...`. Git cannot keep a bare `main` or `dev` ref and child refs beneath the same name. Do not create bare namespace refs such as `dev-windows` or `main-desktop`, because they would block topic branches beneath those prefixes.

Always branch from a freshly fetched remote baseline:

```bash
git fetch origin
git switch -c main-windows/example origin/main
git branch --set-upstream-to=origin/main
```

Use the matching `origin/dev` commands only for upstream-dev-based work. Name new branches for the feature or fix, without agent or model names; existing topic names do not override their approved baseline. Push the topic without changing its baseline upstream:

```bash
git push origin HEAD
```

Do not use `git push -u` for topic branches.

## Commits and pull requests

Use a one-line [Conventional Commit](https://www.conventionalcommits.org/) title. Keep the commit and pull request limited to the stated concern, and complete the pull request template with explicit scope and validation evidence.

## Validation

Run the smallest relevant local checks before opening a pull request:

- formatting;
- focused tests and `check` for the affected target;
- `git diff --check`.

Strict Clippy, release/package builds, and platform matrices belong in CI where the required toolchain and operating system are available.

Report evidence precisely. Source review and local checks, GitHub Actions, and real-device validation are separate evidence levels. If a check was not run, write `Not run` and explain why.

## Merging and releases

Coordinate branch cleanup with the maintainer after merge; do not delete branches or discard uncommitted work without authorization.

Public release notes describe user-visible behavior. Keep commit hashes and dependency provenance in CI output and diagnostic manifests, not in public release copy.
