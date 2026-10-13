# Focused vs full-body oversized brief path — fixed-corpus comparison (2026-10-13)

Harness: `crates/argos-osint-core/src/intel_recon/focused_compare_tests.rs` (`focused_compare_report`, run with `cargo test -p argos-osint-core --lib focused_compare_report -- --ignored --nocapture`; debug build, box CPU). Six fixed oversized stories (~131 KB serialized, 1,445 sentences each) carry five gold sentences each: the concluding claim, a quoted attribution, a pronoun follow-up, an alias mention in a later paragraph and a late denial at the end of the story. The assertion test `focused_path_on_fixed_corpus_reduces_requests_and_keeps_gold_context` runs in the normal suite.

| story | full req | focused req | full bytes | focused bytes | packet coverage | gold kept |
| --- | --- | --- | --- | --- | --- | --- |
| port-talks | 3 | 1 | 130772 | 5285 | 14 of 1445 sentences | 5/5 |
| ferry-halt | 3 | 1 | 130826 | 5314 | 14 of 1445 sentences | 5/5 |
| budget | 3 | 1 | 130784 | 5298 | 14 of 1445 sentences | 5/5 |
| grid | 3 | 1 | 130818 | 5322 | 14 of 1445 sentences | 5/5 |
| mayor | 3 | 1 | 130763 | 5279 | 14 of 1445 sentences | 5/5 |
| audit | 3 | 1 | 130747 | 5268 | 14 of 1445 sentences | 5/5 |

Local packet build (180 runs): cold (sentence index + packets) p50 37.70 ms, p95 38.82 ms; warm (packets only) p50 28.87 ms, p95 29.61 ms.

**What this shows:** Inferences requests drop from 3 to 1 and request bytes by about 96%, and every gold context sentence (conclusion, attribution, pronoun, alias, late denial) stays in the packet.

**What it does not show:** no live model ran. Missed conclusions, attribution errors and missed counterevidence in *generated* output, and provider queue/first-output/generation latency, are not measured. The corpus is synthetic and easy, with distinctive actor names and filler that matches nothing. The focused path therefore stays **opt-in** (`defaults.focused_oversized_brief = false`) until a live held-out quality set clears it.
