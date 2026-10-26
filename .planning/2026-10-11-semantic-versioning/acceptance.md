# Semantic-versioning setup acceptance

> Maintenance note (2026-10-24): retired code-index tooling notes were removed; other recorded results are unchanged.

Assessed 2026-10-11 for implementation commit `fca363a`. This is a historical
acceptance record, not a claim that subsequent revisions have passed these checks.
The maintained release contract and activation steps are in
[docs/versioning.md](../../docs/versioning.md).

## Checks and results

- `node --test .opencode/tests/*.test.mjs`: all 21 local tests passed.
- `node --test .opencode/tests/release-policy.test.mjs .opencode/tests/tools-registry.test.mjs`:
  all 13 portable CI tests passed, including a rerun before the commit.
- `node scripts/release_check.mjs versions`: the workspace, local lockfile entries,
  version mirror and manifest agreed at 0.1.0.
- Prepared message validation and `git diff --cached --check` passed.
- Release Please 17.3.0 was exercised with the actual manifest/config and a mocked
  read-only GitHub client: fix/perf produced patch, feat and breaking 0.x produced
  minor, stable breaking produced major, chore/docs/revert produced no release PR,
  and an explicit stable promotion produced 1.0.0. The feature proposal changed
  only the intended versions/changelog; inherited crate versions, formatting,
  comments and external dependencies were preserved.
- Official configuration schema validation and actionlint 1.7.12 passed for both
  new workflows; optional shellcheck/pyflakes integrations were disabled.
- Skill validation passed. After OpenCode reload, resolved permissions and the
  repository-scoped live skill/command APIs exposed `argos-commit` and `ecc-commit`.

The dependency-free Node checks above can be rerun directly from the repository
root. The external updater simulation and schema/actionlint checks used temporary
tooling, not a maintained test harness; rerunning them requires equivalent tool
versions and a read-only mocked client. Raw tool output is local evidence.

## Limits and remaining setup

No Rust source or dependency changes were made, so Rust compilation/Clippy/tests
were not rerun for this setup. No tag or release was published. GitHub API
authentication returned 401; remote merge settings, branch protection and App/PAT
credentials were not verified. Complete the GitHub setup in the maintained guide
before relying on unattended release PR checks/publication.
