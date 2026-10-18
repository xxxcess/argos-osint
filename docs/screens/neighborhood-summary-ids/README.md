# Neighborhood summary: rejected-key banner and host-note line

Actual-cell captures from `dump_brain_topology_screens` (`scripts/tui_review.py capture`, includes `topology-summary-rejected-key-*`, `topology-summary-generated-*`, `topology-summary-host-note-*`) at 120x40 and 80x24, fictional fixture data.

Reviewed by Grok Bot (an agent) by opening each PNG; no human or terminal-emulator pass. What was seen:

| Image | Seen |
| --- | --- |
| `topology-summary-generated-120x40` | Summary tab, green `generated · mock/summary-model` label, Overview/Connections/Interpretation text, scroll indicator `rows 1-18 of 32`, Relationships panel below. |
| `topology-summary-generated-80x24` | Same label, Overview text and the `Connections` heading; the rest needs scrolling (`rows 1-7 of 23`). Relationships shows its tabs and one row. |
| `topology-summary-rejected-key-120x40` | Amber `Summary · rejected` label and banner naming `unknown reference "b23"` and the allowed form; recorded brief (Overview) below; `Retry summary` and `View logs` buttons. |
| `topology-summary-rejected-key-80x24` | Same banner, wrapped on three rows with `"b23"` visible; the recorded brief shows only its title line before the scroll indicator (`rows 1-6 of 28`), so its body needs scrolling. Buttons visible. |
| `topology-summary-host-note-120x40` | Generated label, then one dim line `Ignored unknown reference b23` above the brief title; text, scroll indicator and Relationships intact. |
| `topology-summary-host-note-80x24` | Same dim line directly under the label, above the brief title, visible without scrolling; Overview text still visible. |

Not seen or not claimed: the images use canned fixture text and a mock model name, not a real model output; font rendering is an approximation of a terminal.
