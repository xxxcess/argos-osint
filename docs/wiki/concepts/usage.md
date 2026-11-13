# Use Fleetworks

[Concepts](index.md) · [Quick start](../../../README.md#quick-start) · [Service guide](../code/fleetworks.md)

[![Provision, connect and register a workstation](../../diagrams/native-connections.svg)](../../diagrams/native-connections.html)

**Native setup:** Gateway URL → operator approval once → macOS Keychain → authenticated reconnect. Enter the independent gateway URL in Setup Gateway and share its request ID/code with the operator. The app saves approved access and registers this installation; no file management is needed. Explicit operator/headless clients retain token-file mode. [Pairing/recovery](../code/native-connections.md#pairing-and-keychain-agreed-target) · [Verification](../../../.planning/2026-11-12-native-pairing-implementation/acceptance.md)

The opaque workstation Connect ID also selects the browser’s separately authorized session. IP is optional metadata. Native launch starts no services or child folders.

## Intel

Intel Web opens at `/connect`. **Bulletin** browses retained articles; **Focus Brief** shows source/actors/claims/inferences/Related leads; **Topology** exposes provenance with an accessible list twin. A Connect ID is not a credential. Native can close while the browser and durable work continue. [Web contract](../code/intel-web.md)

## Native destinations

[![Six destinations and the selected view](../../diagrams/native-workstation-layout.svg)](../../diagrams/native-workstation-layout.html)

| Surface | Use it for | Contract |
| --- | --- | --- |
| DB Seed & Train | Configured actor seed/index/Natural candidate triggers; no picker or activation | [Readiness and maintenance](../code/native-connections.md#readiness-and-maintenance) |
| News Cycles | New Cycle, workstation history and association Delete; shared evidence/audit stays intact | [Atlas setup](../../../deploy/atlas-dispatch.md) |
| Brain Search | One search field, bounded canonical results, selected dossier; no native graph | [Actor discovery](../code/actor-search.md) |
| OSINT Tools | Service/provider cards and protected key modal; no manual tool execution | [Keys and choices](../code/native-connections.md#configuration-and-model-choices) |
| Model Assignments | Configured provider keys, supported routing/variants and ordered fallbacks | [Providers](providers.md) |
| Jobs & Logs | Workstation queue and selected-job safe summaries, without tabs/filters/action toolbars | [Activity/privacy](../code/native-connections.md#durable-requests-and-activity) |
| Status, via gateway header | Native identity, gateway/readiness and optional API-host resources | [Control allowlist](../code/native-design-system.md) |

<a id="atlas"></a><a id="brain"></a><a id="jobs-and-logs"></a><a id="profile"></a>

## State and recovery

Acceptance means queued, not completed. Unknown outcomes retain the original request/body/context for read-only recovery. Membership/private eligibility precede activity; there is no global fallback. Blank key fields preserve values; explicit Clear removes only its slot. Closing a view stops reads, never jobs.

The protected installation retains settings, credential references and unresolved journals. Existing child folders are historical and remain intact; backend deployment files/stores have separate ownership. [Identity](../code/native-workspaces.md) · [Storage](../code/storage.md)

Venture, terminal clients and legacy `argos` are [retired](../../archive/cli-retirement.md). Their records are historical, not startup instructions.
