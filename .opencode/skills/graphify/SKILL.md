---
name: graphify
description: Query the project graph before broad codebase exploration; build or refresh it when requested.
---

# Graphify in Argos

For a codebase question, run `graphify query "<question>"` first when `graphify-out/graph.json` exists. Use `graphify explain "<symbol>"` for a focused concept and `graphify path "<A>" "<B>"` for relationships. Search the returned paths to verify source details. If the graph is absent or unusable, use scoped source search and explain how to bootstrap it.

For `/graphify --help`, show these commands and stop:

```
graphify query "<question>"
graphify explain "<concept>"
graphify path "<A>" "<B>"
graphify extract . --code-only --cargo
graphify update .
```

The Argos graph is built without an LLM: `graphify extract . --code-only --cargo`. Generate the text report with `graphify cluster-only . --no-viz --no-label`. After code edits, use `graphify update .`. Commit `graph.json`, `manifest.json`, and `GRAPH_REPORT.md`; keep caches, analysis, and visualization local.

Read detailed references only for the operation at hand:

- [Query, path, and explain](references/query.md)
- [Extraction options](references/extraction-spec.md)
- [Incremental updates](references/update.md)
- [Hooks and watch mode](references/hooks.md)
- [Exports](references/exports.md)
- [GitHub and merged graphs](references/github-and-merge.md)
- [Add and transcription](references/transcribe.md)
