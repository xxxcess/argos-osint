# Native readiness, bootstrap, and pagination acceptance

**PR base:** `nightly`, confirmed by the user. The phase status below is the tracked handoff; local working notes remain in `task_plan.md`.

| Phase | Scope | Status |
| --- | --- | --- |
| Implementation | Readiness repair controls, actor seed on bootstrap, compact paged cycle/job/log surfaces | Complete in source; behavior awaiting CI |
| Static and contract checks | Formatting, diff hygiene, scoped compilation | Formatting and diff hygiene passed; local compile interrupted by user direction (exit 130); first CI planning-link failure repaired in `b8ce79c4` |
| Feature functionality | Targeted readiness, bootstrap, pagination and client tests | `b8ce79c4` CI identified an obsolete native pager test and generic-bootstrap LFS pointer failure; fixes pending next CI revision |
| Native review | Rebuild isolated backend and native app, close old app, open new app, inspect behavior | Pending after CI |
| Broader gates | Hosted Linux/platform/storage and installed-app evidence as selected | Pending |

The user directed this session to use GitHub CI for builds and tests, with local compilation only when native review is needed. No local build or test result is claimed. Native review checks the pinned size and SHA-256 of the materialized Git LFS actor dataset before Docker build or service stop, mounts it read-only, and sets `FLEETWORKS_BOOTSTRAP_ACTOR_CSV`; bootstrap checks the same fingerprint before ingestion. Generic bootstrap without that variable initializes storage without actor seeding. The runtime image does not bundle the dataset. Cycle, job and log pages use bounded cursor contracts.
