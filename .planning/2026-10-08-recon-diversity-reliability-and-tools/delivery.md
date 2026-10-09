# Delivery: Recon diversity, reliability, Whoxy, and Holehe

Spec: `/Users/successndalama/Downloads/ARGOS_RECON_DIVERSITY_RELIABILITY_AND_TOOLS_SPEC.md` (audit 2026-10-09, HEAD at spec time `ae9d47f`). Plan: `2026-10-08-recon-diversity-reliability-and-tools`. Date: 2026-10-09.

Offline fixture and workspace tests passed. No credentialed live engine, Whoxy, or Holehe success is claimed.

## Root cause and fix

| Finding | Fix |
| --- | --- |
| Answer/repair used one synthesis secret; empty output looked like provider-unavailable on resume. | Shared `recon/model_exec.rs` runs the configured role chain through `provider_chain` once per logical request. |
| Resume treated any assistant message as a finished answer. | Explicit draft/final state; unique non-null `final_message_id`; resume looks up final state. |
| Active `run_turn` never ran mandatory discovery; Intel collect used one Firecrawl search. | Shared `tool_runner` + `diversity` plan; two distinct eligible tools per relevant category; 2 queries × 3 named engines when broad discovery is required. |
| `TurnClock` terminated phases on elapsed time. | Remaining/hard-limit/tools-blocked are non-controlling telemetry. Per-request timeouts stay. CLI `--turn-seconds` / `--max-turn-seconds` are hidden with a deprecation warning. |
| Intel leases claimed once then awaited without renewal. | Schema 26 `lease_epoch`; conditional claim; renew every 30s; owner+epoch fencing. |
| Firecrawl `/v2/search` has no engine selector. | `firecrawl_google_search`, `firecrawl_yandex_search`, `firecrawl_mojeek_search` scrape engine SERP URLs via `/v2/scrape` with per-engine parsers. |
| WHOIS history was missing. | `whoxy_whois_history`: IDNA domain, full-history cache, local `from`/`to`/`limit` projection, prepaid pool (`PlanInterval::Never`). |
| Email-registration coverage was missing. | Native Holehe adapters (Twitter, Spotify, Pinterest) plus a 123-id catalog. GPL sources were not copied. |

## Changed files (high level)

New:

- `crates/argos-osint-core/src/recon/model_exec.rs`
- `crates/argos-osint-core/src/recon/tool_runner.rs`
- `crates/argos-osint-core/src/recon/diversity.rs`
- `crates/argos-osint-core/src/osint/search_engines.rs`
- `crates/argos-osint-core/src/osint/whoxy.rs`
- `crates/argos-osint-core/src/osint/holehe/{mod.rs,catalog.rs,twitter.rs,spotify.rs,pinterest.rs}`

Also updated: `provider_chain.rs`, `provider_attempt.rs`, `provider_diag.rs`, `provider.rs`, `recon.rs`, `orchestrate.rs`, `picker.rs`, `budget.rs`, `clocks.rs`, `tool_io.rs`, `osint.rs`, `contracts.rs`, `results.rs`, `store.rs`, schema SQL, Intel persist/worker/synthesize/jobs, CLI, TUI `app.rs`/`ui.rs`, `Cargo.toml` / `Cargo.lock` (`scraper`, tokio `test-util`), catalog-count tests (66 tools / 67 ids with Hunter alias), docs (`AGENTS.md`, `architecture.md`, `providers.md`, `concepts.md`, `usage.md`), `graphify-out/{graph.json,manifest.json,GRAPH_REPORT.md}`.

## Tests

Command order: `fmt` → `clippy` → `test` (offline, `ARGOS_EMBED` unset).

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --offline
graphify update .
```

Results (2026-10-09, after catalog-count and clippy fixes; workspace re-checked after fmt):

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `cargo test --workspace --locked --offline` | pass: `argos-osint-bin` 99 passed / 3 ignored; `argos-osint-core` 562 passed / 6 ignored; 0 doc-tests |
| `graphify update .` | pass: 6531 nodes / 16095 edges / 237 communities |

CI MiniLM (`ARGOS_EMBED=1`) was not run here.

## Supported Holehe catalog

Upstream pin: `14da70f588538936b20d238783c5e28a0772a2b3` (123 modules, excluding `__init__.py`).

| Service | State | Fixture | Live | Notes |
| --- | --- | --- | --- | --- |
| `twitter` | experimental | yes | no | GET `api.twitter.com/i/users/email_available.json`; exact boolean `taken` |
| `spotify` | experimental | yes | no | GET `spclient.wg.spotify.com/signup/public/v1/account?validate=1`; status 20 / 1 |
| `pinterest` | experimental | yes | no | GET `www.pinterest.com/_ngjs/resource/EmailExistsResource/get/`; exact boolean `resource_response.data` |
| remaining 120 | unsupported | no | no | Known IDs return `unsupported`; unknown IDs are input errors |

Default-enabled is withheld until positive/negative live validation with user-controlled addresses. Instagram, GitHub, Docker, and WordPress stay unsupported in this increment.

## Remaining live-validation gaps

Bounded uncertainty only:

- External endpoint availability (Google/Yandex/Mojeek SERP HTML, Whoxy, Twitter/Spotify/Pinterest email endpoints) can change after the audited HEAD.
- Named-engine adapters have parser fixtures. Three credentialed smoke tests with neutral queries are still required before claiming reliable live coverage. CAPTCHA/consent/block HTML is a coverage gap, never another engine.
- Whoxy has fixture coverage (success/empty/error/IDNA/filter/credit/redaction). No credentialed live balance or history call was made.
- Holehe adapters have exact positive/negative/unknown/429 fixtures. No live email checks were made.
- The original empty-synthesis incident’s live provider response was not reproduced.
- Diagram SVG labels still say `user_version 24`. Prose docs now say schema 26.
- `TurnClock` / `format_deadline` remain as telemetry and persisted columns; they no longer terminate work.

Catalog arithmetic: 61 pre-spec tools + 3 Firecrawl engines + Whoxy + Holehe = **66** unique registry ids; Hunter alias makes the UA id list **67**.
