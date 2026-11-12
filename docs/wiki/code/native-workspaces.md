# Native workspace ownership

[Fleetworks](fleetworks.md) · [Native connections](native-connections.md) · [Protected manager](../../../crates/fleetworks-workspace/src/native/)

The native container is `~/.fleetworks` under the operating system's account home. The root is a protected container, never a selectable workspace. Its protected `default` child is created on first use with stable local identity. Additional workspaces are validated immediate children. Backend storage, credentials, provider accounts and model roles are not copied into children.

[![Local identity and shared backend resources](../../diagrams/workspace.svg)](../../diagrams/workspace.html)

## Filesystem boundary

The manager holds directory descriptors and checks actual file identity. It rejects symlink substitutions, external paths, unsafe permissions, missing/conflicting markers and case/Unicode aliases that collide with another child. Names are one basename with canonical normalization and explicit bounds. Existing unknown directories and historical operator roots are repair states, not automatic adoption or recursive cleanup targets.

Create is private and create-new. Attach accepts an existing owned child with matching markers. Remove from app and physical deletion are separate operations. The default child cannot be removed, retired or renamed. Missing default identity is a repair condition; startup does not silently recreate a competing identity.

The manager serializes catalog changes with its private lock and journals pending operations. Unknown registration outcomes persist across process restart and block removal. Selecting another child changes future origin, not backend-wide settings or already accepted work.

The [scope figure](../../diagrams/workspace-scope.html) contrasts backend-global resources with captured child origin and workstation-only activity.

## Backend lifecycle before removal

A child can have known bindings to more than one backend. Removal snapshots the exact child identity and known bindings. Every relevant backend must provide its actual retirement outcome; an unavailable gateway, pending response or incomplete receipt preserves local bytes and the pending journal. A local folder marker cannot manufacture server retirement authority.

The backend fences new admission, revokes sessions and waits for associated jobs and tasks, including genuine causal descendants, before returning deletion-ready evidence. It preserves Jobs/domain history and keeps old identities distinct from a newly created same-name child. A cancelled HTTP request does not prove that work drained.

Only a completed, identity-checked local lifecycle may proceed to a separately guarded macOS Trash adapter. A cleanup-pending state means bytes remain. Inspect the selected revision's native action/fixture evidence before treating that platform operation as verified; tests use isolated adapters and do not trash the user's workspaces.

[![Protected workspace lifecycle and retirement recovery](../../diagrams/workspace-lifecycle.svg)](../../diagrams/workspace-lifecycle.html)

## Verification

[Native manager integration tests](../../../crates/fleetworks-workspace/tests/native_workspaces.rs) exercise fresh/default identity, direct-child names, remove versus attach, all-backend retirement receipts, restart recovery and link/substitution rejection. [Registry and job-action fixtures](../../../crates/argos-osint-core/tests/storage_job_actions.rs) exercise real admission and retirement fences. Native Runtime/PNG/tree fixtures verify the same UI handlers. Actual Finder/Trash behavior and installed-app launch remain separate matching-hardware checks.
