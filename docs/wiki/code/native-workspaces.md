# Native workstation identity

[Fleetworks](fleetworks.md) · [Connections](native-connections.md) · [Protected manager](../../../crates/fleetworks-workspace/src/native/)

The native installation owns protected settings under `~/.fleetworks` in the operating system account home. Workstation registration uses that installation marker and authenticated backend scope. The gateway-issued opaque `workstation_id` is the Connect ID. IP is optional mutable metadata; changing or omitting it does not create a new identity, and two installations sharing an IP remain distinct.

[![Protected installation and shared backend](../../diagrams/workspace.svg)](../../diagrams/workspace.html)

## Protected settings boundary

Normal startup opens settings without creating a `default` child. The manager holds directory descriptors, verifies object identity and private permissions, and serializes revisioned atomic catalog writes with its existing private lock. Unknown catalogs, symlink substitution, external paths and conflicting installation markers fail closed. The connection catalog stores exact-origin protected credential references, never token bytes.

`--provision-gateway-credential --api-origin <origin> --api-token-file <private-file>` binds an existing protected token file to one normalized origin, exits without a GUI/network/service startup, and preserves other origins. Token rotation requires explicit provisioning again. The URL-only wizard cannot forward another origin's credential.

## Scope and history

New registrations, request contexts and activity queries contain workstation identity without workspace IDs, names or markers. Backend models, keys, actor corpus and search remain shared within authorized deployment scope. Membership is resolved before counts, pagination, details, logs, results, streams and mutations. Unknown/system jobs are never assigned to a workstation as a fallback.

The versioned registry migration preserves workstation IDs and `(scope_id, installation_hash)`, retains old registration mappings and immutable origins for historical readers, fences incompatible admission and requires a fresh browser Connect. Accepted causal children retain the correct workstation lifecycle fence. Legacy child registrations cannot admit new work.

[![Backend resources and workstation origin](../../diagrams/workspace-scope.svg)](../../diagrams/workspace-scope.html)

Uncertain submissions retain original operation/request ID, exact body/context and original registration. Authenticated read-only reconciliation uses `original_registration_id`; an unknown lookup keeps the unresolved handle and never sends a replacement POST under a newer registration.

## Legacy folder records

Existing default/named child folders, markers, pending operation journals and unresolved receipts are preserved. They are historical local data, not the active identity or a prerequisite for setup. The native interface exposes no folder creation, attach, selection, removal, Finder or Trash workflow. The retained manager compatibility paths do not authorize deleting user folders during this migration. Cargo workspaces, deployment directories and Jobs' private stores have separate ownership.

[![Settings, provisioning and registration recovery](../../diagrams/workspace-lifecycle.svg)](../../diagrams/workspace-lifecycle.html)

## Verification

Manager fixtures cover settings-only initialization, installation identity, credential-origin/file binding and retained catalogs. Registry/API tests cover forward migration, IP-independent identity, session invalidation and historical request recovery. Actual Rune fixtures exercise the same production views/handlers. See the [acceptance tracker](../../../.planning/2026-11-12-native-workstation-redesign/acceptance.md) for executed results; unit fixtures do not establish installed-app or both-architecture approval.
