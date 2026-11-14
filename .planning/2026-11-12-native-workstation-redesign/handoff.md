# Native workstation redesign — handoff

**State:** product changes implemented;120 scoped library tests pass, full embedded runtime gate blocked by disk capacity. [Acceptance/results](acceptance.md) · [Docs audit](phase8-design.md) · [Backend audit](phase9-design.md)

[![Native setup and independent backend](../../docs/diagrams/native-connections.svg)](../../docs/diagrams/native-connections.html)

## Read next

| Need | Maintained authority |
| --- | --- |
| Product/navigation/control allowlist | [Native design system](../../docs/wiki/code/native-design-system.md) |
| Credentials, registration, requests and activity | [Connections](../../docs/wiki/code/native-connections.md) · [Protected identity](../../docs/wiki/code/native-workspaces.md) |
| Independent backend operation | [Deployment guide](../../deploy/README.md) |
| Native build/captures | [App README](../../apps/fleetworks-desktop/README.md) · [Verification](../../docs/wiki/code/native-verification.md) |
| Original detailed requirements | [Bundle README](../../docs/design/native-workstation-handoff/README.md), then implementation specification and design guide |

## Implemented work

| Concern | Owned paths / behavior |
| --- | --- |
| Identity and migration | Contracts/core/API: v2 opaque workstation ID, optional IP, forward migration, preserved legacy origins and fenced inactive admission |
| Native setup | Protected installation settings, exact-origin credential-reference provisioning, URL-only setup, durable registration/deletion recovery; no hidden child or service startup |
| Maintenance and cycles | Configured revision-bound inputs, Natural candidate-only training, bounded cycle reads and workstation-only history tombstones |
| Native UI | Six sidebar destinations, header Status, explicit setup, safe bounded/virtualized activity and allowlisted editors |
| Web | Workstation Connect/session/cache migration, logout/stream authority fences, matte theme and canonical branding; structure retained |
| Build helper | Flushed stages/live Cargo stderr, unchanged artifact stdout, safe optional Launch Services request |
| Docs | Shorter diagram-led guides and source-backed file/diagram audits |

Root owns integration/Cargo and shared acceptance. User authorized incremental commits. Build-progress helper committed as `241af22`; the accompanying feature commit records coordinated product/docs changes. No push/deployment has been performed. Source is frozen; both demonstrated precommit blockers and the Runtime assertion are repaired. Current user data, running stacks/apps, frontend locks and unrelated `dist/` remain preserved.

## Finish line

1. Final scope is frozen. Native/contracts/manager tests pass120/120; final formatting and workspace/all-target Clippy with warnings denied pass.
2. Full embedded gate stopped safely at <3 GiB free reserve, exit130 before runtime; preserve that blocker and rerun the documented required gate after sufficient capacity is available.
3. [Acceptance](acceptance.md) records final evidence and unavailable hosted/other-hardware/installed-GUI checks. No runtime-storage approval is implied by the source commit.

Existing relevant-test authorization persists. Use the pinned1.95.0 toolchain, `scripts/agent_cargo.py … --locked`, one build owner and the shared debug graph. Native app opening is explicit; no backend deployment or hosted dispatch is implied. Follow [scope/freeze policy](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line).
