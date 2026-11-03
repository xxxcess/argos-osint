# Code Architect review — Intel web CI

The user authorized adding the prepared workflow while retaining Code Architect review. Review [intel-web.yml](../../.github/workflows/intel-web.yml); existing Rust workflows are unchanged.

The workflow uses read-only repository permissions, checkout/setup-node v7, the pinned Node/pnpm toolchain, a frozen dependency install, typecheck/lint, an API-independent static export, and offline Playwright fixtures. It neither deploys the application nor accesses provider credentials. The JavaScript tests exercise connection gating, source-fenced topology seeds, mutations and 320px/wide layouts.

YAML syntax and frontend commands pass locally; the Node policy suite passes 21/21. A hosted Actions run has not been requested or performed. Rust/API/process validation is recorded in the maintained [implementation spec](../../docs/argos-intel-living-spec.md), which is the milestone authority.

Review the new workflow and its trigger paths after publishing the implementation. No commit, push, PR or hosted run of the implementation is part of this handoff.
