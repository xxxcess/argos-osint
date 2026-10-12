# Atlas classification and Related articles

Atlas's `atlas-collection-v1` cycle retains native news discovery, country heat bands, regional coverage and article deduplication. Classifier is its only model role. Cycles do not extract claims, assess fidelity, generate briefs, publish Brain memories or require vector indexing. Intel and Recon retain their independent on-demand investigative workflows and publication services.

[![Atlas cycle](diagrams/atlas-pipeline.svg)](diagrams/atlas-pipeline.html)

## Classification and native metadata

Stable article batches default to eight rows and are cut against a 32 KiB serialized request ceiling. Source IDs, titles, descriptions and publishers stay separate. Native Decisions uses one question ID per article; chat uses an array of exact article IDs and category IDs. The six existing categories and `unk` are valid results. Pending, running, classified, failed and blocked are separate receipt states; absent configuration and malformed replies never become successful `unk`.

The shared model executor owns admission, route fallbacks, retry waits and cancellation. Valid partial answers survive while only unresolved IDs are requested again inside that captured retry budget. Batch receipts include input revisions, Classifier routes and prompt version. Resume reuses successful unchanged articles. Empty cycles request no model.

A bounded native inventory processes complete original title and description fields as articles are retained, including `unk` articles. Its local parsing overlaps a single publisher lookup and later Classifier batches. Mentions retain spelling, compatible type, normalized comparison key, source field and UTF-8 byte spans. Explicitly disabled or uncompiled NLP is unavailable; no model fallback runs. Empty success, unsupported language and parse failure remain distinct.

Publisher priors use `wikipedia_source_reliability`, normalized URL host and longest valid domain suffix. A lookup pass reuses an immutable WP:RSP index. The index has a 30-day durable TTL, bounded single refresh and last-good behavior. **Initial confidence · publisher-based** is a source prior, never verified article accuracy: A 100, B 85, C 55, D 30, E 12; F has a null score. Information credibility stays unassessed (6). Intel's later evidence confidence remains separate.

## Related leads and storage

Related means **shared headline/description entities**. Local posting lists create canonical undirected article pairs with exact compatible person, organization or place mentions. At least one endpoint must have a recognized non-`unk` category; two `unk` rows alone cannot seed a pair. Verified source spans, complete-name Unicode boundaries and input revisions constrain matching. Publisher/country metadata alone cannot create a pair. People/organizations rank before place-only matches, then shared count and rarity; ordering is browsing priority, never confidence.

Schema 30 adds `atlas_article_metadata`, `atlas_article_mentions` and `atlas_related_articles`. Article/run content remains authoritative; stale revisions hide associations, and deletion removes dependent metadata without deleting independently retained Brain evidence. Related pages are bounded, reciprocal and carry true totals and coverage. Local reason text such as `Shared mentions: Acme, Jane Doe` grants no corroboration, premise or assessed graph edge.

Intel places Related below assessed Links. Brain's Related pane separates Memories from Articles for article-derived insights. Lineage uses exact derivation evidence and legacy source run/call IDs, with canonical explicit URLs as fallback; prose similarity never supplies lineage. Opening a row selects that exact article's Intel Focus Brief, including `unk`, and preserves the prior view for Back. Loading a list makes zero model calls.

## Completion, recovery and compatibility

Collection/classification is required. Enabled metadata settles independently; disabled metadata is skipped, parser/lookup failures give warnings and unavailable coverage. Valid `unk`, empty entities and unrated publishers are successful outcomes. Missing Classifier configuration blocks required classification; exhausted unresolved IDs yield partial or failed cycles. Pause preserves receipts and Resume retries unresolved work. Native acquisition receipts commit with their statistics, so a failed acquisition can resume without replaying successful source requests or double counting discovery pages.

Old unfinished extraction/indexing cycles resume through retained classifications and native metadata. Ambiguous historical `unk` is flagged as legacy provenance rather than bulk reclassified. Historical insights and shared indexing services remain readable. Atlas-owned obsolete refresh/repair work is fenced at upgrade; Intel/Recon work continues. Explicit CLI Repair/Verify remains for historical contracts and rejects `atlas-collection-v1`. The Atlas TUI no longer exposes Repair memories or startup reconciliation.

Profile retains `atlas.backlog`, `atlas.cycles` and `atlas.temperature`. Current telemetry counts articles and metadata coverage; overlapping stage counts are not summed. Historical memory units are explicitly legacy. Running Atlas alone no longer fills Brain.

## Verification

`atlas_classify_tests` uses offline loopback providers to check both transports, exact IDs, partial replies, cancellation, fallback routes, revisions, cold/warm receipts and actual wire attempts. Its fixed 17-article workload compares singleton dispatch with eight-row batches and records executor queue/execution/backoff, local metadata and matching times. The comparison uses the shared executor on both sides; timing samples under workspace-test contention are descriptive, not production throughput estimates. A cached native index supplies publisher observations; live WP:RSP refresh latency is not measured by this workload.

Metadata/Related fixtures check concurrent classification writes, source spans, unknown categories, incompatible types, revisions/deletion, reciprocal bounded pages and exact multi-source lineage. Independent Intel/Recon publication and indexing fixtures remain in the workspace gate.

Run formatting, isolated strict Clippy and offline workspace tests with locked dependencies and no default features, then repeat with `--features nlp`. For fresh TestBackend captures use `scripts/tui_review.py`: `dump_atlas_related_screens` covers Coverage, Intel Related, both Brain tabs and explicit empty/loading/unavailable states; `intel_brief_stacked_left_column_renders_all_sections_and_scrolls` covers publisher/evidence confidence; `analytics_viewports_and_data_states` covers Profile Atlas. Select the affected stems at 160×50, 120×40, 100×32, 80×24 and 60×18. Inspect the emitted PNGs and record visual, interaction and metric results separately under the named local plan, following [TUI verification](tui-verification.md). Live providers, a live terminal, default LanceDB builds and MiniLM checks require separate evidence.
