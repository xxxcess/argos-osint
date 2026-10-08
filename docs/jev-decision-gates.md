# Decision roles and output contracts

A single `DecisionContract` compiles to:

1. **Jev-native questions** — `typesafe/jev-1.13` via OpenRouter `POST /alpha/decisions`
2. **Schema-constrained prompts** — `StrictJsonSchema`, `JsonMode`, or `ValidatedText`

Application code owns surface restrictions, budgets, credentials, and recovery. Models plan and synthesize.

[![Tool picker transport](diagrams/tool-picker.svg)](diagrams/tool-picker.html)

## 12-template registry

| Template | Role | Focus | Allowed outcomes |
|---|---|---|---|
| `mode` | `classifier` | Which deliverable? | `verify`, `explain`, `assess_outlook`, `full_assessment`, `ambiguous` |
| `directive_alignment` | `investigation_controller` | Does candidate.objective advance task.objective? | `aligned`, `unrelated`, `insufficient` |
| `tool_selection` | `tool_picker` | Which candidate supplies required evidence? | Tool IDs, `no_match`, `insufficient` |
| `scarce_provider_need` | `investigation_controller` | Is distinctive provider evidence still needed? | `necessary`, `unnecessary`, `insufficient` |
| `evidence_relevance` | `evidence_curator` | Does the passage address the subject? | `relevant`, `wrong_subject`, `unrelated`, `insufficient` |
| `extraction_fidelity` | `evidence_curator` | Is the observation faithful to the passage? | `faithful`, `unsupported_addition`, `material_omission`, `insufficient` |
| `entity_binding` | `entity_resolver` | Same identity? | `same_entity`, `different_entity`, `ambiguous` |
| `claim_relation` | `claim_assessor` | How does the passage bear on the claim? | `supports`, `contradicts`, `mentions_only`, `irrelevant`, `insufficient` |
| `handoff_adequacy` | `investigation_controller` | Are findings and unfinished work preserved? | `adequate`, `omission`, `unsupported_addition`, `insufficient` |
| `next_action` | `investigation_controller` | Which task closes the gap? | Task IDs, `request_replan`, `defer`, `insufficient` |
| `resolution` | `investigation_controller` | Is the answer established, disputed, incomplete, or blocked? | `resolved_supported`, `resolved_disputed`, `needs_evidence`, `blocked`, `insufficient` |
| `publication` | `claim_assessor` | Is the conclusion faithful? | `faithful`, `overstated`, `contradictory`, `insufficient` |

## State isolation

`DecisionState` treats retrieved passages as quoted data, not instructions:

- `task` — id, revision, objective, required evidence, completion criteria
- `subject` — confirmed bindings and ambiguity sets
- `candidate` — action, observation, binding, handoff, or conclusion
- `evidence` — source IDs and verbatim passages
- `computed_checks` — freshness, schema, eligibility, budget
- `missing_context` — known omissions

## Adapters

| Adapter | Behavior |
| --- | --- |
| `NativeDecisions` | OpenRouter `POST /alpha/decisions` (`choice`, `noul`, `score`). Never `/chat/completions`. |
| `StrictJsonSchema` | OpenAI-compatible structured outputs; required keys only |
| `JsonMode` | `response_format: json_object`, local schema check |
| `ValidatedText` | JSON-only instruction + local AST parse |

## Thresholds

`ThresholdPolicy`:

- Winning probability default 0.50
- Top-1 vs top-2 margin default 0.10
- `insufficient` / `ambiguous` / `no_match` → `EnforcementOutcome::Uncertain`
- Only `Pass` approves. `Uncertain` and `Unavailable` do not progress.

## TUI

- Defaults pane: active model, inheritance, adapter (`native_decisions`, `strict_json_schema`, …)
- Live: `Deciding… · <model> · <gate>` then `Decision · <summary> [<outcome>] · <model>`
- Jev has no thinking tokens. Other-model reasoning stays in collapsed thinking rows.
