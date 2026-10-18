# Local NLP preprocessing

Stored article metadata is the initial Actors source. Focus Brief validates metadata hashes and exact original title/description UTF-8 spans, retaining typed names, spellings and repeated references. Dates, amounts and currencies are excluded. Displaying Actors never runs another NLP pass or Entity Resolver, and unavailable/unsupported/failed NLP has no model fallback. Atlas phase 6 records validated source-local Actors and observed associations; headline candidates do not publish Claims/Inferences or run vector indexing.

On-demand analytical refresh requires complete retained acquisition coverage, matching body revisions and no truncation. Title/description are supplied with the entire cleaned baseline using separate source references and correctly translated mention offsets. Refinement/display excerpts cannot replace or certify the baseline. Report refresh uses a separate immutable report-only input with complete stage coverage and recall disabled; it has no article-body gate or independent raw-tool-root requirement. Assessment receipts preserve original lineage without treating report prose as independent corroboration. Both modes use the existing refresh queue and canonical publication/index services.

**Intel brief cleanup (Stage 1).** The order is local baseline → NLP annotation (cached by prose hash) → conditional Classifier → local verbatim removals → revalidation, span maps and content hash (`intel_recon::clean_body`). NLP mentions in the body that the title or description also names become **keep-anchors** (`nlp_anchors`), along with title/description proper-noun tokens. A short block naming such an entity (for example "the harbour master said…") is kept without a model decision. Anchors only add keep-confidence; they never make a block removable. Without NLP, title/description tokens remain the anchors and Stage 1 completes with a diagnostic.

Argos embeds pinned Compromise 14.18.0 and dates 3.9.0 through rquickjs 0.14.0. No Node, browser, loader, network or subprocess is used at runtime. Candidate tags rank source passages; existing identifier, ownership, claim and publication gates remain authoritative.

Standalone calendar months tagged Person are excluded in temporal contexts (for example “in August” or a matching retained date span). Full names such as August Wilson and May Chen remain eligible. The guard applies to article Actors, ingest Related overlap and investigation entity preparation. Failed article metadata retains bounded parser flags as well as status, so unavailable inventory can be diagnosed without triggering a model fallback. Explicit NLP cache connections use initialized lightweight Stores and never repeat migrations or attach vectors.

[![NLP evidence flow](diagrams/nlp-preprocessing.svg)](diagrams/nlp-preprocessing.html)

## Actor label maintenance

The elected actor maintenance service sends up to 100 existing Actor labels to the same bounded NLP worker before starting its SQLite write transaction. A batch has a two-second parser deadline; labels over 1024 bytes and account/domain/IP identifiers retain lexical normalization only. The parser trims verbal boundary context only when a clear entity remains: `Allow FBI` and `FBI announced` become `FBI`. Ambiguous people and phrases such as `May Johnson`, `Will Smith`, `Save the Children` and `Human Rights Watch` remain unchanged. Original aliases, IDs and exact source spans survive; cleanup does not establish cross-source identity. Changed text queues index work. Versioned receipts revisit older labels; busy/failed parsing retries on a later tick. NLP Off or a build without NLP keeps lexical normalization, with a separate receipt version so enabling NLP revisits those rows.

The shared annotation queue now accepts bounded actor-label batches as well as source annotations. JavaScript stays on the parser thread, and actor cleanup never adds a provider request. See [actor identity maintenance](architecture.md#actor-identity-maintenance).

## Configuration and build

The local `config.toml` setting is independent of portable Profile schema v1; importing a Profile preserves it:

```toml
insight_recall_limit = 20 # optional root setting, clamped to 10–20 total candidates

[nlp]
mode = "auto" # auto | off | annotate | select
```

`auto` is the default. Typed `PreparePolicy` reserves a serialized prose budget and declares whether coverage is optional or complete. `Prepared` carries verbatim text, source-local candidate entities, coverage, selection and the effective action. Auto keeps all prose when it fits; optional pressure can select original passages. Complete coverage always retains the whole source and requires windows when oversized, including with NLP disabled. Empty or unsupported entity discovery falls back to native/model discovery. Explicit `off`, `annotate` and `select` remain supported; annotate retains its legacy optional prose path. Both crates default to `lancedb` and `nlp`; CI verifies NLP enabled and disabled with LanceDB disabled.

## Evidence and ownership

`Source` separates acquisition and derived prose hashes, retrieval attempts and revisions. UTF-16 offsets from the bridge are converted against the exact immutable chunk into half-open UTF-8 ranges; surrogate-interior, reversed and out-of-bounds spans fail closed. Scalar converters remain explicit for the existing body-filter API.

Article acquisition is saved before validation. Refinement retains its exact attempt and acquisition hash; generic historical body/Brain rows have derived provenance and no manufactured acquisition hash. Firecrawl bounded observations carry an artifact reference; Recon reads complete page text from the retained raw result before preparing compaction and synthesis inputs.

Migration 28 adds `nlp_artifacts`, `nlp_source_refs` and `nlp_features`, and stores complete summary-result envelopes. Owner deletion/pruning removes references and garbage-collects artifacts/features only after their last reference disappears. Feature keys cover canonical JSON, prose hash, pinned bundle/bridge hashes, schema, lexicon/chunk/span policy and date context. Selection keys additionally cover purpose, focus, byte budget and policy. Only complete validated annotations enter the cache: memory is bounded to 64 MiB and disk features to 256 MiB by LRU.

## Runtime and fallback

One process-shared dedicated thread owns the JS runtime/context. Tokio channels carry owned Rust DTOs and a response oneshot; no JS values or database connections cross threads. Admission allows 32 queued jobs and 16 MiB of queued/active input, with a 2 MiB source ceiling, 4 MiB output ceiling, 128 MiB JS heap and 1 MiB stack. Chunks target 4 KiB, never exceed 8 KiB and overlap a bounded adjacent sentence. UTF-8 splits of oversized sentences are flagged. Guards interrupt at one second per chunk and ten seconds per document and observe cancellation. Engine faults trigger recreation; a worker panic fails pending jobs. Incomplete/unsupported/busy/oversized/cancelled/failed parses never authorize selection.

The bridge obtains one sentence/term tree, then returns only compact candidate spans. Money spans include adjoining scale/unit terms. Date phrases are deduplicated and limited to 32 per job and 256 characters each; overflow is retained as unparsed evidence. Relative dates use publication date, otherwise retrieval date, UTC by default. Unknown numeric date order and invalid explicit calendars have no normalized event. No document mutates the global lexicon.

## Passage and model contracts

Optional selection keeps complete original paragraphs in source order with explicit separators. Paragraph units preserve sentence attribution, quotation closure and adjacent context. Literal focus, candidate-entity, correction/uncertainty and numeric/date/quote signals rank passages; opening context and correction units are protected. If a protected unit exceeds the budget, selection returns `BudgetExceeded`. Complete-coverage work retains all evidence and uses overlapping windows regardless of NLP mode; missing windows withhold inferences rather than substitute a bounded prefix.

Budgets are serialized UTF-8 bytes, including JSON escaping and message overhead, rather than token counts. Existing smaller field/output budgets remain. Selected and omitted sentence IDs make partial coverage explicit. Body claims are accepted against only the transmitted windows of that exact article artifact, plus title/description evidence; the maximum remains eight accepted body claims. Cleanup requests in select mode demand exactly one Boolean decision per reviewed block ID. Missing, duplicate, unknown or malformed IDs preserve the baseline; unreviewed blocks always remain. Rust assembles retained source characters verbatim and revalidates the body.

Summary identity covers all ordered sources, supplied/computed hashes, metadata, focus, budget, required fields, prompt version, preprocessing and resolved provider/model. Lookup and insertion use the same `sum-<key>` ID. Deterministic fallback identity is local and separate from the durable upgrade route; an upgrade cannot be overwritten by a deterministic result. Flush requests retain original sources and immediate fallback separately. New contracts use v2; queued v1 envelopes remain readable and explicitly execute as legacy derived sources. All-source hashes and actual result route/model are stored. Summary execution has two total outbound attempts, including transport fallback, with unchanged admission.

## Integration inventory

| Path | Preparation and evidence behavior |
| --- | --- |
| Article acquisition/refinement | Persist raw acquisition; cache whole prose; use prepared effective action for strict block decisions; retain unreviewed blocks and baseline on failures. Report workers retain captured settings/routes/cancellation |
| Atlas title/description | Complete local candidate mentions including unk articles; source spans and comparison keys; no model fallback or claim publication |
| Firecrawl / Recon pages | Reference full acquisitions separately from bounded observations; prepare before compaction, then reuse for synthesis and claim mining |
| Recon bindings | Run native extraction first; focus existing model requests; retain occurrence, ownership, platform and format gates |
| Directives / prior / Brain | Preserve mandatory question/directive/state fields; prepare optional prose without adding requests |
| Intel sections / BLUF | Read full body/upstream sections before focused body input; preserve evidence, elements, stances and objectives |
| Summaries | Original source envelopes; canonical all-source/route identity; durable fallback and coverage manifests |

V1 bypasses Atlas category/title-description classification and eligibility, peer/report-mode classifiers, actor review, picker/catalog state, opening query generation, short titles/link explanations, graph structures, remote Firecrawl URL extraction, audio transcription and MiniLM embeddings. These inputs are short, typed, remote or already scoped. NLP candidates never create bindings, identities, truth scores, graph edges or credibility categories.

## Verification

Offline unit fixtures cover the actual JS runtime/plugin, exact Unicode spans, repeated mentions, scaled amounts, relative/ambiguous/invalid dates, interrupts, admission bytes, disconnected channels, immutable provenance/shared-owner GC, canonical keys, tail corrections after 48,000 bytes, quoted context, invalid cleanup IDs and summary upgrade protection. The gold selection fixture reports source/selected bytes and omitted IDs; runtime fixtures report parsing and memory-cache latency without document text. Scripted provider tests retain native gates, mandatory searches, admission and retry/transaction checks. Actual provider token usage and live workload latency require a separate workload run; synthetic fixture numbers are not throughput SLOs.

The structured JSON gold input produces invalid offsets in the pinned parser. Its acceptance path explicitly checks `Failed`, rejects selection and retains the original bounded source with a diagnostic. The multilingual fixture similarly checks `Unsupported` and safe fallback. English entity misses remain valid data; they do not remove material evidence. The quotation fixture covers an opening attribution before the focused paragraph and a closing delimiter after it.

### Earlier implementation fixture measurements (2026-10-11)

Historical measurements below concern the earlier three-mode implementation. They are not a speedup claim for the shared assessment/window pipeline; full coverage and semantic assessment can add requests.

| Fixture | Observed result |
| --- | --- |
| Compressible article, correction after 48,000 bytes | 55,404 source bytes → 322 serialized selected bytes; 799 omitted sentence IDs; partial coverage; all marked material spans and attribution retained |
| Scripted summary, 53,711 source bytes | Off and annotate: 12,878 serialized request-body bytes each; select: 709 bytes (94.5% less); one outbound request per mode; late denial and attribution retained |
| Actual runtime, 88-byte date/money fixture | Cold engine initialization: 1,393 ms; uncached parse in initialized engine: 551 ms; queue wait: 0 ms; memory-cache response: 21 ms; serialized annotation: 864 bytes |

These are single observations from a concurrent offline test suite on the local host. They establish byte/count and evidence acceptance, not throughput or latency targets. No live provider token usage was available. The small gold set exercises clean/noisy English, missing entities, numerical disputes, multilingual prose, code and structured extracts; it does not establish broad entity accuracy or production workload savings.

The earlier implementation validation passed `cargo fmt --all --check`, both locked no-default-features Clippy lanes with `-D warnings`, and both workspace test lanes with `ARGOS_EMBED` unset. NLP enabled: 187 binary, 724 core and 3 fixture checks passed. NLP disabled: 187 binary, 719 core and 3 fixture checks passed. Each lane retained eight ignored tests. CI includes both lanes on the existing macOS Intel/ARM and Linux x86/ARM matrix; those remote runs are separate from this local validation.

## Investigative coverage and publication

The shared `investigation::insights` service runs source claim extraction through Claim Assessor, assesses fidelity with finite labels, checks readiness, then runs Synthesis over original evidence windows. Candidate entities are source local and preserve exact forms and byte mentions; they do not establish identity or support. A negative classification with a passing threshold does not authorize publication.

`insights-v3` collects all source windows before canonical deduplication and owner-wide selection (eight article/report claims, five Recon claims, five inferences and eight links). Negation, modality, attribution and time remain distinct; late denials receive priority when enumeration exceeds the cap. Limits are disclosed and never certify that omitted candidates or source ranges were analyzed by assessment. Both extraction and Synthesis still analyze every required range.

Semantic assessments submit at most eight ID-addressed items with a shared dictionary keyed by source revision and exact span/pointer. Each dynamic question inherits its original gate policy; same-name identity batches retain the Entity Resolver route. Claim fidelity and own-source relation are separate answers in one batch; the original source creates no independent support receipt. Report receipts come exclusively from the immutable completed bundle. Native Decisions and chat contracts require every submitted question exactly once; a confident negative label remains negative. Request fit uses the compiled native and chat envelopes, with room for repair, rather than state size alone.

Model proposals cite compact evidence IDs and **source-absolute**, half-open UTF-8 byte spans (not window-relative offsets), or exact JSON pointers. Native pointer windows include their byte span in the canonical JSON artifact, including preceding siblings; entity mentions validate within that window and decode JSON escapes. Rust hydrates acquisition metadata, provenance, canonical IDs, snapshot identity and initial assessments, and rejects unknown IDs or citations outside supplied windows. Proposal text is concise; the original source is never prefix-cut. Multi-window reconciliation revisits each original window with bounded retained proposals, preserving prior limitations and late corrections. Its checkpoints are derivations, not new evidence roots.

Request measurements, compatibility migration and scoped visual checks are recorded in the [insight reliability acceptance](../.planning/2026-10-12-insight-reliability/acceptance.md).

Atlas collection-v1 supplies complete original title and description to local candidate inventory only. It makes no claims, fidelity, inference or publication requests; see [Atlas classification and Related](atlas-classification-and-related-articles.md). Full article reasoning requires retained acquisition evidence, complete Claim Assessor coverage and complete Synthesis coverage; rewritten display text cannot certify completeness. Recon requires completed required tasks, at least two substantive roots with different artifacts, and a faithful cited premise. Search snippets and duplicate acquisitions do not satisfy the gate. Caps disclose partial enumeration and do not stop window coverage.

Schema 29 adds article/Recon owner generations, immutable snapshots, derivations, source and premise evidence, typed links and stage checkpoints. Exact-task refresh workers renew leases and publish the canonical Brain rows, provenance, active pointer and index outbox in one fenced transaction. Successful stages are reused; exhausted chains are not silently restarted. Pending and failed generations retain previous history. Legacy extraction jobs keep their explicit contract marker.

## Implementation measurements (2026-10-11)

### Progressive Intel confidence

Focus Brief retains the ANSI Shadow Admiralty badge and shows Source, Information and Brief rating beside/below it. The standalone Assessment card is removed. `brief-rating-v2` maps known source scores A/B/C/D/E to 100/85/55/30/12 and information 1/2/3/4/5 to 100/82/58/35/15; F/6 are missing, not zero. Initial ratings average known axes, cap source-only at 60, and keep F6 N/A.

Typed passed support/contradiction receipts identify exact retained passages and lineage. Independent roots collapse transitively by shared origin, artifact hash or canonical URL. Claims with 1/2/3+ verified roots use evidence strength 60/80/95, with weight `d/(d+2)` capped at d=3 against the mean of known publisher/information scores. Inferences cannot exceed 85 or their weakest supported premise, and cannot exceed information grade 2. Report assertions without embedded verified support receipts remain 40/information 6/provisional. The aggregate averages all unique supported claims/inferences after multiplying each raw score by `supported_claims/(supported_claims+contradicted_claims)`, rounding only the final mean. Conflicts grade 4, all-refuted grade 5/rating 0, unresolved/partial grade 6, empty completed assessment N/A. Legacy snapshots without typed receipts remain unassessed. Current snapshots retain policy, source identity and raw scores; pending/failed replacements display the previous rating. No scoring model or render-time work is added.

The following measurements describe the earlier 2026-10-11 implementation, rather than validation of the current policy.

The final locked feature lanes passed formatting, strict Clippy and workspace tests with `ARGOS_EMBED` unset: 188 binary tests in each lane, 739 core tests with NLP disabled, 744 with NLP enabled, and three search fixtures in each. Each lane has nine ignored tests. Scoped visual review inspected 44 current Intel confidence/lifecycle captures and nine Brain captures. Existing map/icon font fallback and live terminal/provider behavior remain outside that visual certification.

The separate default-feature vector lane passed both cached MiniLM tests: paraphrase ranking and LanceDB/Store retrieval of the expected memory ID. The restored LanceDB-only Atlas publication rollback/retry fixture also passed; its missing test imports/helper alias were repaired without changing production behavior.

Four separate processes used fresh state directories, identical short and pressure evidence, complete coverage and a 512-byte preparation budget. Each workload used ten cold and ten warm samples. Cold means the first use of a source, not restarting the engine for every sample. The short workload includes initial engine startup; pressure follows with the initialized engine. Warm repeats reuse the exact annotation. Complete coverage retains all original evidence, so pressure requires downstream windows instead of selection. Times below measure preparation, excluding benchmark cache preflight and model work.

| Workload | Mode | Cold p50 / p95 (ms) | Warm p50 / p95 (ms) | Serialized Prepared bytes | Warm cache hits |
| --- | --- | --- | --- | --- | --- |
| Short | off | 0.047 / 0.127 | 0.030 / 0.041 | 360 | 0 |
| Short | annotate | 435.780 / 1,624.439 | 22.586 / 29.589 | 800 | 10 |
| Short | select | 381.020 / 1,586.083 | 20.436 / 26.059 | 800 | 10 |
| Short | auto | 451.981 / 2,137.918 | 27.829 / 56.372 | 800 | 10 |
| Pressure | off | 0.359 / 0.473 | 0.380 / 0.411 | 3,204 | 0 |
| Pressure | annotate | 143.821 / 200.123 | 24.698 / 31.469 | 3,648 | 10 |
| Pressure | select | 131.652 / 174.007 | 23.582 / 35.763 | 3,648 | 10 |
| Pressure | auto | 184.853 / 222.182 | 31.720 / 36.393 | 3,648 | 10 |

All 160 preparation checks retained the original assertions, attribution, denials and corrections with complete coverage. Outbound model requests were zero in every preparation workload, a zero request delta from off. Parser p50 across cold and warm samples was zero because cache hits perform no parse; parser p95 was 453/440/627 ms for short annotate/select/auto and 155/130/178 ms under pressure. Measured queue p50/p95 was zero at millisecond resolution. Off bypasses parsing and has no parser/queue measurement.

The claims-only Atlas fixture independently verifies one extraction request and one fidelity request, zero inference/context/link-generation requests, and zero additional requests on identical replay. The scripted summarization fixture verifies one request per mode and retention of late denial and attribution. Provider tokens and production model latency were not measured. These fixtures establish coverage and request behavior; they do not establish a stack-wide speedup percentage or a throughput target.

For the 53,711-byte scripted summary source, serialized request bodies were 12,878 bytes in off and annotate, and 709 bytes in select and auto. Each mode made one request, a zero request delta from off. This optional selection fixture is separate from the complete-coverage preparation workloads above.

## Classifier-owned cleanup and mode decisions (2026-10-13)

Article/prompt mode classification and page cleanup run through the complete Classifier chain. With a native Jev route they are bounded decisions (questions keyed by stable mode / block IDs, strict labels); otherwise the first chat-capable fallback receives the equivalent JSON contract. Cleanup answers keep/remove per known block ID and Rust computes the irrelevant ranges, so retained text stays verbatim. Memory predicate/time relevance runs on Evidence Curator; identity checks on Entity Resolver; assessment batches are partitioned by owning role (≤8). Figure: [recon-turn](diagrams/recon-turn.html).

## Claim-specific Intel inference evidence (2026-10-16)

`intel_recon/brief_context.rs` selects original sentences from the shared `SentenceIndex` for each extracted claim. It matches detected actor names and observed spellings on token boundaries, associates actors with the claim, and retains nearby attribution/pronoun context and correction/denial candidates. Claim event terms add relevant event-detail candidates even without a repeated actor name; nonmatching text stays local, while Classifier rejects unrelated lexical matches and protected-qualification candidates. Every locally matched candidate reaches Classifier; there is no pre-classification sentence-count cap. Candidate and omitted sentence IDs remain traceable to the retained clean article and its UTF-8 spans.

The Classifier role evaluates small bounded decision batches for one claim, labeling each sentence support, context, counterevidence or irrelevant. The adapter supports native Decisions and constrained JSON chat. Synthesis analyzes one claim, associated actors and selected original sentences, including counterevidence. Every extracted claim receives an analysis request; empty evidence permits an empty conclusion. Classifier batches contain at most eight sentences and 8 KiB of serialized input. Final analysis deduplicates identical sentence/relation pairs and retains counterevidence before support/context within 12,000 evidence bytes and 20,000 system-plus-user bytes, recording omissions. Oversized atomic sentences or counterevidence alone exceeding the bound fail explicitly, and there is no full article chunk fallback. Support fidelity reviews both cited passages and all selected counterevidence for the assigned claim, including denials the generation output omitted; receipts cover the complete assessed passage set. Original support and counterevidence spans remain provenance. Semantic deduplication follows. The selection policy, prompt and captured Classifier route participate in inference revision identity.

`passage_packets.rs` remains available for historical callers and the offline [2026-10-13 comparison](brief-focused-comparison-2026-10-13.md). Its old opt-in oversized-story flag no longer selects the automatic brief's inference path. That historical comparison measured request shape and gold-context retention, not live model quality or the latency of this claim-specific pipeline.
