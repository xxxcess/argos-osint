# Searchable tags

[Wiki](../index.md) · [Maintenance](maintenance.md)

Tags connect a documented concern to its implementation across directory boundaries. Use literal lowercase `@tag:<slug>` tokens with hyphenated slugs. Add them sparingly to maintained docs and module/function comments, never runtime strings. A tag describes a stable concern, not a task, author or temporary status.

| Token | Documentation | Source anchor |
| --- | --- | --- |
| `@tag:recon-flow` | [Recon](code/core.md#recon) | [recon.rs](../../crates/argos-osint-core/src/recon.rs) |
| `@tag:recon-bindings` | [Bindings](code/core.md#recon) | [tool_io.rs](../../crates/argos-osint-core/src/recon/investigation/tool_io.rs) |
| `@tag:intel-brief` | [Intel](code/core.md#intel) | [brief_pipeline.rs](../../crates/argos-osint-core/src/intel_recon/brief_pipeline.rs) |
| `@tag:atlas-collection` | [Atlas](code/core.md#atlas) | [atlas.rs](../../crates/argos-osint-core/src/atlas.rs) |
| `@tag:brain-memory` | [Brain](code/core.md#brain) | [brain.rs](../../crates/argos-osint-core/src/brain.rs) |
| `@tag:osint-providers` | [Providers](code/core.md#providers-and-tool-adapters) | [providers.rs](../../crates/argos-osint-core/src/osint/providers.rs) |
| `@tag:surreal-storage` | [Storage](code/storage.md) | [storage/mod.rs](../../crates/argos-osint-core/src/storage/mod.rs) |
| `@tag:tui-components` | [Interfaces](code/interfaces.md) | [components.rs](../../crates/argos-osint-bin/src/tui/components.rs) |

From the repo root:

```sh
rg -n '@tag:recon-bindings' docs crates
rg -n '@tag:[a-z0-9-]+' docs/wiki crates
```

Reuse a registered tag before adding one. New tags need a row here, a meaningful doc occurrence outside this registry, and at least one source comment. When renaming/removing a concern, search the old token and update both ends and inbound links in the same change. Tags complement path links and graph queries; they do not replace source verification.
