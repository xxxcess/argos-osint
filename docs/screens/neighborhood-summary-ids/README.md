# Neighborhood summary: rejected-key banner

Actual-cell captures from `dump_brain_topology_screens` (`scripts/tui_review.py capture`, includes `topology-summary-rejected-key-*` and `topology-summary-generated-*`) at 120x40 and 80x24, fictional fixture data.

- `topology-summary-rejected-key-*`: a generated summary rejected because the model cited `b23`. The Summary pane banner now names the offending key (the wrapper text "model operation stopped: … brief rejected:" and the attempt bookkeeping are stripped; the reason is capped at 240 characters). The recorded brief, Retry summary and View logs stay available.
- `topology-summary-generated-*`: a generated summary for comparison.

Visual review (images read by an agent, no human or terminal-emulator pass): banner wraps instead of clipping, key visible at both sizes, Retry/Logs buttons present, no overlap or border damage. At 80x24 the Relationships panel stays visible.
