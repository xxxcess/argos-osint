---
name: graphify
description: Query the repository graph before broad searches; verify results against source.
---

When `graphify-out/graph.json` exists, start repo questions with shell `graphify query "<question>"`; verify returned paths against source. Use `graphify explain "<symbol>"` or `graphify path "<A>" "<B>"` for focused relationships. No separate graphify tool exists.

If absent/unusable, use scoped grep/glob and explain bootstrap: `graphify extract . --code-only --cargo`, then `graphify cluster-only . --no-viz --no-label`. After code edits run `graphify update .`; retain graph.json, manifest.json and GRAPH_REPORT.md, not caches/visualizations.

For `/graphify --help`, list query/explain/path/extract/update and stop. Load references only as needed: [query](references/query.md), [extraction](references/extraction-spec.md), [update](references/update.md), [hooks](references/hooks.md), [exports](references/exports.md), [GitHub/merge](references/github-and-merge.md), [watch](references/add-watch.md), [transcription](references/transcribe.md).
