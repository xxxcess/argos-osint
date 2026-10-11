# Versioning, commits and releases

Argos uses [Semantic Versioning 2.0.0](https://semver.org/) and
[Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).
One version covers the CLI/TUI and core crate. `Cargo.toml`'s
`workspace.package.version` is the application version; both crates inherit it
with `version.workspace = true`, and `argos --version` uses that package version.
Database schema numbers and provider/API/model versions are separate contracts.

The initial baseline is **0.1.0**. No historical tag or release is implied.
`release-please-config.json` records a bootstrap commit, excluding earlier
history from the first generated changelog. After the first generated release,
Release Please tracks subsequent releases instead of the bootstrap boundary.
Do not move that boundary to hide unreleased work.

## Commit and PR convention

Use `<type>(optional-scope)!: description`, omitting `!` for compatible changes.
Types and scopes are lowercase. Prefer a short behavior-focused subject; use a
body to explain the problem, resulting behavior, validation and material limits.
Suggested scopes include `tui`, `recon`, `intel`, `atlas`, `brain`, `osint`,
`models`, `profile`, `store`, `providers`, `cli`, `opencode`, `release`, `deps`
and `graph`. Comma-separated scopes are supported when one coherent change
crosses surfaces. Scopes describe the change; they do not select a crate version.

| Change | Type | Release impact |
| --- | --- | --- |
| New user or library capability | `feat` | Minor |
| Correct existing behavior, including security/dependency fixes | `fix` | Patch |
| Improve runtime or workflow efficiency | `perf` | Patch |
| Internal restructuring without behavior change | `refactor` | None by itself |
| Documentation, tests, build, CI, maintenance, formatting | `docs`, `test`, `build`, `ci`, `chore`, `style` | None by itself |
| Revert housekeeping | `revert` | None by itself; use `fix` when restoring user behavior |
| Incompatible public change, with any type | `!` and `BREAKING CHANGE:` | Minor during 0.x, major from 1.0.0 |

Before 1.0.0, compatible features still bump minor and compatible fixes bump
patch: 0.1.0 → 0.2.0 for a feature or breaking change, 0.1.0 → 0.1.1 for a fix.
From 1.0.0, features bump minor and breaking changes bump major. The highest
impact across unreleased changes wins. Maintenance commits join the next release
without causing one themselves; release notes emphasize features, fixes,
performance and compatibility changes.

Public compatibility includes CLI commands/flags and machine-readable output,
core library APIs, persisted configuration/portable import formats, user data
readability and documented workflows. An additive database migration that
preserves existing data is normally compatible; a schema counter increment is
not automatically a breaking change. TUI layout refinements are normally fixes
or features; removal of a documented workflow needs compatibility assessment.
Never use `chore` to conceal user-facing fixes or breaking changes.

```text
fix(recon): preserve paused transcript position during updates

Keep the retained history anchor when incoming synthesis adds content.
Validated with the transcript follow/history regression tests.
```

```text
feat(cli)!: replace the legacy export payload

BREAKING CHANGE: Export now emits the portable configuration schema.
Consumers must migrate their parser to the documented fields before upgrading.
```

This repo requires a nonempty `BREAKING CHANGE:` footer for `!` subjects,
including PR titles; `BREAKING-CHANGE:` is also supported. Footer-only breaking
changes are recognized, but agents should include `!` to make them visible.
Keep the footer in the final squash commit body. `Release-As: X.Y.Z` is reserved
for an explicitly requested maintainer release operation, such as promotion to
1.0.0; agents do not invent an override or bump every commit's version.

## Agent commit tasks

Read this policy when asked to commit, prepare a PR or change release files.
Inspect `git status` and the current staged diff, preserve unrelated/pre-staged
work, and stage exact task files or hunks. Inspect the entire staged diff before
committing. Do not use `git add .` for a dirty shared workspace. Stage only the
relevant portions of mixed files; leave unrelated generated graphs/evidence out.
An editor reports compatibility impact to its parent; the parent owns commits.

Classify planning artifacts using [planning-artifacts.md](planning-artifacts.md).
Include relevant maintained shared docs and curated durable deliverables; keep session
working memory, generated evidence and host selectors local. Do not force-add
ignored files or stage the whole `.planning/` directory.

Use a task-owned scratch message file with actual newlines, then run:

```sh
node scripts/release_check.mjs message --file .agent-scratch/<task>/commit.txt
node scripts/release_check.mjs versions
git diff --cached --check
git commit --file .agent-scratch/<task>/commit.txt
```

Node 20+ is required; the checker has no npm dependencies. It checks commit
syntax, breaking footers and synchronized versions, and reports release impact.
It does not decide whether a change really breaks compatibility or replace the
Rust fmt → Clippy → test gate, scoped TUI review or other relevant checks. Agents
record actual validation in the commit body and report the SHA. Existing task
authorization governs commits, pushes, merges and releases. The read-only
OpenCode `plan` agent prepares messages/impact only; it does not stage or commit.

OpenCode `build` and `plan` can load `argos-commit`; `/ecc-commit` routes to it.
`argos-plan`, `argos-implement`, editors and reviewers carry the same policy.
Never manually edit release versions/changelog for an ordinary feature/fix.
Release Please owns their synchronized update. Explicit release/baseline repair
tasks may change them together and must pass the checker.

## Release automation

[Release Please](https://github.com/googleapis/release-please-action) runs on
pushes to `main`, or manual dispatch on `main`. It collects eligible commits
since the last release and maintains a release PR. Merging a reviewed, green
release PR authorizes its `vX.Y.Z` tag and GitHub Release on the next run.
There is no automatic merge and no crates.io publishing or binary distribution
in this workflow. Existing Rust CI remains the build/test gate.

The release PR updates:

- The canonical workspace version in `Cargo.toml`.
- Only the two local workspace package versions in `Cargo.lock`.
- `version.txt`, the simple strategy's synchronized mirror.
- `.release-please-manifest.json`, the automation's synchronized release state.
- `CHANGELOG.md` and GitHub release notes.

The `simple` strategy plus explicit
[TOML updaters](https://github.com/googleapis/release-please/blob/main/docs/customizing.md#updating-arbitrary-toml-files)
preserves inherited crate versions. This virtual workspace has no root
`[package]`, so the built-in Rust updater is not used. Adding a crate requires
updating the lockfile selector and version checks together. The selector uses
`@.name.value` because Release Please’s TOML parser wraps string values with
source-position metadata; this is tested against its actual updater. The lightweight
checker reads the explicit version tables; Cargo CI remains the full TOML and
dependency-resolution check. `node scripts/release_check.mjs versions vX.Y.Z`
also checks a prospective tag against the workspace version.

The **Commit policy** workflow validates PR title/body on creation, edits and
updates; validates every new commit pushed to `main`; checks version equality;
and runs the offline release-policy and agent-registration tests. Feature branches can contain
WIP commits because their final squash commit is the release input. Agents
still use Conventional Commits for their own commits. Historical commits before
the workflow are not rewritten. Workflow actions added here are pinned to full
SHAs; review pins when updating them.

## GitHub setup and first release

Repository settings are external to these files; merging this setup does not
change them. Configure them once:

1. Enable squash merging. Set the squash title default to **PR title** and the
   message default to **PR body**. Protect `main` with required reviews, the
   **Conventional commits and shared version** check and all four existing
   platform CI checks. Prefer disabling merge/rebase merging to keep the
   inspected PR title/body as the release input. If another merge mode is used,
   every commit reaching `main` must follow this policy; merge-generated subjects
   are intentionally rejected.
2. Enable **Allow GitHub Actions to create and approve pull requests** under
   Actions settings when using the default token. Prefer a dedicated GitHub App
   installed only on this repository, with contents, issues, pull-request and
   workflow write access. Set repository variable `RELEASE_APP_ID` and secret
   `RELEASE_APP_PRIVATE_KEY`; the workflow mints a fresh repository-scoped token
   each run and revokes it afterward using
   [actions/create-github-app-token](https://github.com/actions/create-github-app-token).
   Alternatively, set a fine-grained PAT as `RELEASE_PLEASE_TOKEN` with those
   permissions. Workflow permission lets an existing release branch incorporate
   changes to workflow files. Token preference is App → PAT → default token.
   Do not store credentials in files.
3. With only `GITHUB_TOKEN`, release PRs/releases can be created, but their events
   do not trigger normal CI. A human must close and reopen the release PR to
   start its required checks. The workflow warns about this fallback. Configure
   the App or `RELEASE_PLEASE_TOKEN` for unattended release PR CI; see the
   [action's token guidance](https://github.com/googleapis/release-please-action#token).
4. Merge ordinary work with a valid Conventional Commit title/body. Review the
   generated release PR's proposed version, migration notes, unchanged external
   dependency versions and inherited crate manifests. Let CI finish, then merge.
   Confirm the matching `vX.Y.Z` tag and GitHub Release exist afterward.

For the initial stable 1.0.0 release, explicitly assess API/config/data guarantees
and migration documentation, then use a maintainer-requested releasable commit
with `Release-As: 1.0.0`. Do not retroactively tag the baseline or rewrite shipped
tags. If automation fails, rerun it after fixing the cause; avoid manually moving
release labels/tags or resetting its manifest without a release-repair task.
