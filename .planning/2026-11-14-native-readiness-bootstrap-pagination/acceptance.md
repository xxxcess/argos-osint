# Native readiness, bootstrap, and pagination acceptance

**PR base:** `nightly`, confirmed by the user. [Task plan](task_plan.md) tracks the working phases.

| Phase | Scope | Status |
| --- | --- | --- |
| Implementation | Readiness repair controls, actor seed on bootstrap, compact paged cycle/job/log surfaces | Complete in source; behavior awaiting CI |
| Static and contract checks | Formatting, diff hygiene, scoped compilation | Formatting and diff hygiene passed; local compile interrupted by user direction (exit 130) |
| Feature functionality | Targeted readiness, bootstrap, pagination and client tests | Pending GitHub CI |
| Native review | Rebuild isolated backend and native app, close old app, open new app, inspect behavior | Pending after CI |
| Broader gates | Hosted Linux/platform/storage and installed-app evidence as selected | Pending |

The user directed this session to use GitHub CI for builds and tests, with local compilation only when native review is needed. No local build or test result is claimed. Native review mounts the materialized Git LFS actor dataset read-only into bootstrap; the runtime image does not bundle it. Cycle history uses bounded offset pages ordered by creation time and ID; a newly inserted cycle can shift later pages during browsing. Jobs and logs use existing cursor contracts.
