# Profile dashboard: chart choices per KPI

> Historical record. For current behavior, start at the [wiki](../index.md).

The Profile → Overview dashboard (`crates/argos-osint-bin/src/tui/profile*.rs`,
data from `argos_osint_core::profile_stats::ProfileSnapshot`, which reads the
`telemetry` event table plus live store state) has a Summary page and five
section pages, each with a 4-tile KPI strip above 7–8 panels (20 primary views
plus "Needs attention").

## Rules used

| Data shape | Chart | ratatui primitive |
|---|---|---|
| Ratio with a direction (share of N) | Bullet-style gauge, colour = good/neutral/bad band | `Gauge`/`LineGauge` style, 1 row |
| Count per time bucket (headline total) | Sparkline of buckets, missing coverage ≠ 0 | `Sparkline` style, 1 row |
| Percentile per time bucket (p50/p95) | Sparkline in tile; line chart in panel | `Sparkline`; `Chart` / braille line |
| Outcome mix over time | Stacked columns | `BarChart` (stacked) |
| Categorical share (origins, publishers, causes) | Ranked horizontal bars, shared scale | `BarChart` horizontal |
| Distribution over bins (freshness delay) | Histogram with categorical bins | `BarChart` |
| Before/after or paired (confidence, stage time) | Paired / diverging bars | custom bars |
| Pipeline (Atlas discovery: candidates → retained) | Stacked share bar / funnel | stacked bar |
| Live counters / quotas | Gauge (used / max) | `Gauge` |
| Row-level entities (backlog, attention, perf) | Table with selectable rows | `Table` |

## KPI tiles (changed in this PR)

| Tile | Shape | Before | After |
|---|---|---|---|
| Report completion, Answered directives, Atlas completion, Body availability, Completed share, Corroboration coverage, Usable search fetches, Evidence yield | ratio | number only | gauge, green ≥80 / amber ≥50 / red |
| Remote errors, Top-three publisher share | ratio, lower is better | number only | inverted-colour gauge |
| Distinct new articles, Terminal runs, Terminal cycles, Sends, Final operation failures, Logical calls | count over period | number only | sparkline of the section's bucket totals |
| Successful model p95, Queue p95 | percentile over period | number only | sparkline of per-bucket p95 |
| Unresolved · Live, Blocked · Live, Oldest pending · Live, Active/max · Live | live scalar | number only | unchanged (no history to plot) |

Panels already use the matching chart types (stacked columns for outcomes,
line plots for latency, ranked bars, histogram bins, tables) and were left as is.
Suggested next steps: a day×hour heatmap for tool invocations, and a
discovery funnel for `atlas.discovery`.
