# Topology neighborhood summary — screenshots (Phase 3)

Actual-cell captures from `dump_brain_topology_screens` (`scripts/tui_review.py capture`, include `topology-summary-*`), fictional fixture data. Ten representative captures of the full set (generated at 160x50, 120x40, 80x24 and 60x18; generating, blocked, failed, stale, limited, overflow). Re-capture the full matrix with `scripts/tui_review.py capture` (include `topology-summary-*`; sizes 160x50, 120x40, 100x32, 80x24, 60x20, 60x18, 40x20). 60x18 and 40x20 show only the existing too-small notice, so only one is kept.
States: `generating`, `generated`, `blocked` (no usable Summarization model; Retry/Logs/Models), `failed` (rejected; Retry/Logs), `stale` (refresh failed, previous snapshot retained), `limited`, `overflow` (scrolled).

## Visual review (images read by an agent; no human or terminal-emulator pass)

| Check | Result |
| --- | --- |
| 160x50, 120x40, 100x32: Summary/Record tab bar, one-or-two-row state label (now coloured per state), styled brief text, Basis list, indicator, button row | pass |
| 80x24 (Details panel, stacked): tabs, label, scrolled content, `▲▼ rows a–b of n` indicator | pass |
| Failure states keep the recorded brief and show Retry / View logs (+ Models when blocked) | pass |
| Label wraps instead of clipping `limited coverage` / `refresh failed` | pass (defect found and fixed during review) |
| 60x20: Summary reachable but only 2 content rows; scroll indicator and buttons present | pass, cramped by design (Relationships keeps 5 rows) |
| 60x18 and 40x20: explicit "Needs 60x18 content cells (60x20 terminal)" notice with Back | pass; Summary is intentionally not drawn below the mapper minimum |
| Missing-glyph boxes, overlap, border damage | none seen |

Limits of this review: renderer is an approximation of terminal fonts; the generated text in the fixture is canned. Interaction results are recorded separately (below).

## Interaction results (tests, not images)

`cargo test -p argos-osint-bin topology` — default Summary tab; Summary/Record switching with independent Relationships tabs; Basis Enter/click open Record without recentre, depth, cycle or filter change; focus alone never activates; scroll via wheel/PgUp/PgDn/arrows/Home/End with clamped indicator; generated result replaces the fallback atomically without moving focus; obsolete generation/seed/options/basis results discarded; failure labels and Retry keep content; loading, read-error, stale, isolated, limited, no-seed text; history restores tab and scroll; compact Details and too-small Back; `begin_topology_summary` fallback + pending.
