# Fleetworks interfaces

[Code route](index.md) · [Usage](../concepts/usage.md)

Venture owns the local workspace and stack lifecycle. The terminal clients call the versioned API directly. Intel web uses the same API through its authenticated Next.js relay.

| Surface | Source | Contract |
| --- | --- | --- |
| Venture | [Launcher](../../../apps/venture/src/main.rs) · [Workspace](../../../crates/fleetworks-workspace) | [Workspace, lifecycle and storage modes](fleetworks.md) |
| Terminal clients | [CLI suite](../../../apps/fleetworks-cli) · [API contracts](../../../crates/fleetworks-contracts) | [CLI command reference](../../../apps/fleetworks-cli/README.md) |
| Jobs and Logs | [CLI suite](../../../apps/fleetworks-cli) · [API router](../../../crates/argos-osint-api/src/lib.rs) | [Interactive commands](../concepts/usage.md#jobs-and-logs) |
| Intel web | [Web server](../../../apps/web-desktop) · [Intel UI](../../../packages/intel) · [API client](../../../packages/api-client) | [Intel web contract](intel-web.md) |
| Brain and worker roles | [Service host](../../../services/api-host/src/main.rs) | [Service boundaries](architecture.md#fleetworks-service-boundaries) |

[![Fleetworks workspace](../../diagrams/workspace.svg)](../../diagrams/workspace.html)

## Contributor checks

Use the [Fleetworks CI scopes](ci.md#fleetworksyml--scoped-branch-checks) for Venture and the network clients. Use the [browser verification guide](react-ui-testing.md) for Intel web changes. Changes to the retained Ratatui source use its [contributor verification contract](tui-verification.md).
