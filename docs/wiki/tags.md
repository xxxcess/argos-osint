# Searchable tags

[Wiki](../index.md) · [Maintenance](maintenance.md)

Tags connect a documented concern to its implementation across directory boundaries. Use literal lowercase `@tag:<slug>` tokens with hyphenated slugs. Add them sparingly to maintained docs and module/function comments, never runtime strings. A tag describes a stable concern, not a task, author or temporary status.

| Token | Documentation | Source anchor |
| --- | --- | --- |
| `@tag:intel-brief` | [Intel](code/core.md#intel) | [brief_pipeline.rs](../../crates/argos-osint-core/src/intel_recon/brief_pipeline.rs) |
| `@tag:atlas-collection` | [Atlas](code/core.md#atlas) | [atlas.rs](../../crates/argos-osint-core/src/atlas.rs) |
| `@tag:discovery-services` | [Discovery services](code/storage.md) | [article_discovery.rs](../../crates/argos-osint-core/src/storage/article_discovery.rs), [actor_catalog.rs](../../crates/argos-osint-core/src/storage/actor_catalog.rs) |
| `@tag:brain-memory` | [Brain](code/core.md#brain) | [brain.rs](../../crates/argos-osint-core/src/brain.rs) |
| `@tag:osint-providers` | [Providers](code/core.md#providers-and-tool-adapters) | [providers.rs](../../crates/argos-osint-core/src/osint/providers.rs) |
| `@tag:surreal-storage` | [Storage](code/storage.md) | [storage/mod.rs](../../crates/argos-osint-core/src/storage/mod.rs) |
| `@tag:tui-components` | [Terminal fixtures](code/tui-verification.md) | [components.rs](../../crates/argos-osint-bin/src/tui/components.rs) |
| `@tag:async-runtime` | [Architecture](code/architecture.md#async-runtime-safety) | [hardware.rs](../../crates/argos-osint-core/src/hardware.rs) |
| `@tag:process-lifecycle` | [Venture lifecycle](code/architecture.md#process-lifecycle-and-signal-handling) | [compose.rs](../../apps/venture/src/compose.rs) |
| `@tag:api-timeouts` | [Intel web](code/intel-web.md#api-contract) | [lib.rs](../../crates/argos-osint-api/src/lib.rs) |
| `@tag:abort-signal` | [Intel web](code/intel-web.md#async-presentation) | [index.tsx](../../packages/api-client/src/index.tsx) |
| `@tag:testing` | [Testing](code/react-ui.md#react-component-testing-real-component-pattern) | [UI tests](../../packages/ui/__tests__/) / [Intel tests](../../packages/intel/__tests__/) |
| `@tag:look-and-feel` | [Fleetworks look and feel](code/react-ui.md#fleetworks-look-and-feel) | [UI primitives](../../packages/ui/src/index.tsx) |
| `@tag:web-routing` | [Intel web](code/intel-web.md#running-locally) | [navigation.ts](../../packages/intel/src/navigation.ts) / [web adapter](../../apps/web-desktop/app/navigation.ts) |
| `@tag:api-proxy` | [Intel web](code/intel-web.md#running-locally) | [server-api.ts](../../apps/web-desktop/app/server-api.ts) |
| `@tag:live-updates` | [Live update contract](code/intel-web.md#rest-snapshots-and-live-updates) | [Rust live.rs](../../crates/argos-osint-api/src/live.rs) / [Next relay](../../apps/web-desktop/live-relay.mjs) / [browser adapter](../../apps/web-desktop/app/live-transport.ts) |
| `@tag:host-metrics` | [Intel web](code/intel-web.md#api-contract) | [host_metrics.rs](../../crates/argos-osint-api/src/host_metrics.rs) / [client](../../packages/api-client/src/index.tsx) |
| `@tag:react-ui` | [React UI](code/react-ui.md) | [UI primitives](../../packages/ui/src/index.tsx) / [Intel screens](../../packages/intel/src/index.tsx) |
| `@tag:classifier` | [Classifier](code/classifier.md) | [classifier/mod.rs](../../crates/argos-osint-core/src/classifier/mod.rs) |
| `@tag:atlas-taxonomy` | [Atlas taxonomy](code/core.md#atlas) | [atlas_taxonomy.rs](../../crates/argos-osint-core/src/atlas_taxonomy.rs) |

From the repo root:

```sh
rg -n '@tag:intel-brief' docs crates
rg -n '@tag:[a-z0-9-]+' docs/wiki crates
```

Reuse a registered tag before adding one. New tags need a row here, a meaningful doc occurrence outside this registry, and at least one source comment. When renaming/removing a concern, search the old token and update both ends and inbound links in the same change. Tags complement path links and graph queries; they do not replace source verification.
