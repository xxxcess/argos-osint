# Fleetworks interfaces

[Code route](index.md) · [Usage](../concepts/usage.md)

The native macOS app connects to an independently running gateway. Its six sidebar destinations share the selected deployment's resources within the caller's permissions; a protected installation identifies workstation request origin and activity. Workstation Status opens from the gateway header. Intel Web uses a separately provisioned relay credential and a server-issued registered session. Direct Docker Compose owns backend deployment. Venture, the six terminal clients and legacy `argos` are retired; installed-app and matching-hardware evidence remain separate.

| Surface | Source | Contract |
| --- | --- | --- |
| Native macOS app | [Rune app](../../../apps/fleetworks-desktop) · [Bounded SDK](../../../crates/fleetworks-contracts/src/client.rs) | [Connections and context](native-connections.md) · [Protected workstation identity](native-workspaces.md) |
| Backend deployment | [Standalone Compose](../../../deploy/compose.native-review.yml) · [Provisioning helper](../../../deploy/native_review.py) | [Independent deployment and credentials](../../../deploy/README.md) |
| Native Jobs and Logs | [Activity API](../../../crates/argos-osint-api/src/activity.rs) · [Scoped storage](../../../crates/argos-osint-core/src/storage/jobs_view.rs) | Registered workstation activity; independent action permissions and private shared-job restrictions |
| Intel web | [Web server](../../../apps/web-desktop) · [Intel UI](../../../packages/intel) · [API client](../../../packages/api-client) | [Intel web contract](intel-web.md) |
| Brain and worker roles | [Service host](../../../services/api-host/src/main.rs) | [Service boundaries](architecture.md#fleetworks-service-boundaries) |

[![Fleetworks workstation](../../diagrams/workspace.svg)](../../diagrams/workspace.html)

## Contributor checks

Use the [native Runtime/PNG/tree fixtures](../../../apps/fleetworks-desktop/README.md) and [Fleetworks CI scopes](ci.md#fleetworksyml--scoped-branch-checks) for the affected client or service. Native-only builds do not compile backend engines. Use the [browser verification guide](react-ui-testing.md) for Intel Web. Historical terminal captures are preserved under the [retirement disposition](../../archive/cli-retirement.md); they are not native-app evidence.
