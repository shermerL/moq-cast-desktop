# Contributing to MoQCast Desktop

Thank you for helping improve MoQCast Desktop. Keep each change focused on one independently reviewable and reversible concern.

## Branches

- `main` is the stable and release line.
- `dev` is the active development and integration line.
- Create features and regular improvements from `dev`.
- Create stable fixes from `main`. After a stable fix lands, synchronize it into `dev`.

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
git switch -c dev-windows/example origin/dev
git branch --set-upstream-to=origin/dev
```

Use the matching `origin/main` commands for a stable fix. Push the topic without changing its baseline upstream:

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

Delete topic branches after merge. The only long-lived branches are `main` and `dev`.

Public release notes describe user-visible behavior. Keep commit hashes and dependency provenance in CI output and diagnostic manifests, not in public release copy.
