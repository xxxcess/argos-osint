# Changelog

## [0.2.0](https://github.com/xxxcess/argos-osint/compare/v0.1.0...v0.2.0) (2026-10-24)


### ⚠ BREAKING CHANGES

* **storage:** storage is SurrealDB only. SQLite, LanceDB and the vendored protoc are removed, no existing data is carried over, and the app starts from an empty SurrealDB store.
* **providers:** remove the Grok (xAI) provider; old configs load and import with Grok routes dropped and a warning
* **intel:** CatalogEvidence gains optional reasoning metadata. Rust callers using exhaustive struct literals must initialize reasoning or use
* **intel:** TopologySeed adds RetainedInference. Core-library consumers with exhaustive matches must handle this variant or add a wildcard arm before rebuilding. New inference schema outputs also require at least one premise_claim_ids entry; producers must supply the assigned claim ID. IndexState struct-literal constructors must initialize required or use ..Default::default(); older serialized states remain readable. Custom StageBackend implementations must implement classify_context to run the inference stage; its default returns an explicit unavailable error.
* **providers,brain,atlas:** WireMarks now includes dispatch timestamps and no longer implements Copy. Clone values when reused and include the dispatched field or use Default in struct initializers. StageBudget policies and automatic operation budgets now use provider output/reasoning defaults; callers that require explicit ceilings must set RequestPolicy fields directly.
* **tui,recon:** The public JobStatusFilter enum adds Stopped and ALL contains six entries. Downstream exhaustive matches must handle Stopped; Failed now selects failed/partial history, while cancelled/superseded history belongs to Stopped. The manual Clear old jobs TUI workflow is replaced by terminal-status controls; age cleanup runs automatically. Stored states and existing cleanup APIs remain compatible.
* **intel,brain:** StageBackend::generate returns a future that owns its dispatch state instead of borrowing the backend. Custom implementations must remove the receiver's 'a lifetime and move cloned or owned dispatch state into the returned future to support concurrent requests. Existing CLI, configuration, and persisted data formats remain compatible.
* **brain,tui:** Topology public structs gain fields and seed/node-kind enums gain variants; update Rust struct literals and exhaustive matches. Actor seeds now identify exact actors; use ActorGroup for aggregates. Rebuild cached article and group topology keys, which now include revision or membership. Set depth to 1 explicitly to retain the former default. Remove continuation tokens, which are now rejected, and use bounded coverage fields. No database migration is required; see docs/wiki/concepts/topology-mapper.md for migration details.
* **brain,intel,atlas:** Intel article Recon controls and the intel-recon palette command are removed; article report admission, resume/retry and worker entry reject execution. Use Focus Brief or an independent Recon investigation. Brain browsing now exposes Claims and Actor-name groups only; other records remain available to internal recall and storage APIs.
* **brain:** Public core struct literals now require insight policy/progress fields on OperationScope, ExecuteOptions, ChainReport, JobDetail and PipelineOutput. Migrate to OperationScope::new or default-based literals where available, or initialize the documented false/None/empty/zero values. Persisted data and CLI/configuration remain compatible.
* **intel:** Rust consumers must initialize SourceClaim.support_receipts, handle Surface::Report, initialize the new EvidenceConfidence fields (or use Default), and use the String returned by tasks::ensure_service_job as the current service journal ID. Existing persisted claims default to empty support receipts and remain unassessed until refreshed. Operational job history now expires after 72 hours; start fresh runs for retired analysis requests.
* **atlas:** Atlas cycles no longer extract claims, generate cycle briefs, publish Brain memories or wait for vector indexing; the TUI Repair memories workflow is removed. Analyze selected articles in Intel or investigate in Recon. CLI Repair/Verify remains restricted to historical cycles. Core consumers constructing RunStats must adopt its added defaulted contract and coverage fields. Existing saved insights and configuration remain readable.
* **investigation:** Core NLP mode and Prepared contracts now include Auto and typed entity/coverage metadata. Downstream Rust callers must handle Mode::Auto and adapt preparation field access. Missing NLP configuration now selects auto; set [nlp] mode = "off" to opt out. Schema 29 is additive and legacy records, queued jobs and explicit settings remain readable.
* **nlp:** Rust callers constructing SettingsFile, SummaryRequest or SummaryResult literals must initialize the added NLP, preprocessing, fallback and provenance fields. Use Default for NLP/preprocessing, None for the deterministic request fallback, and initialize result source_hashes/provider. Legacy serialized records and portable Profile imports remain compatible.

### Features

* **atlas:** collect classified articles with local related leads ([ce8bbec](https://github.com/xxxcess/argos-osint/commit/ce8bbec93134094df71a81c20f0fc04cce178e77))
* **brain,intel,atlas:** simplify browsing and related associations ([eaf94c7](https://github.com/xxxcess/argos-osint/commit/eaf94c77cc3903cede2b62b260b43d0a30f98265))
* **brain,tui:** add interactive brain memory topology view and scoped analysis ([78dea7c](https://github.com/xxxcess/argos-osint/commit/78dea7c0e2a645bbb70e4fc16407021bdd69c746))
* **brain,tui:** implement scoped topology mapper ([3590ca4](https://github.com/xxxcess/argos-osint/commit/3590ca4461a1a0d8aa8974d478da6715017bcc80))
* **brain:** actor registry, typed relationship writer and cross-cycle backfill ([8641257](https://github.com/xxxcess/argos-osint/commit/864125765f10db424726f9e6370e00e52bbed58a))
* **brain:** finish unified brain graph phase 5 ([4d44872](https://github.com/xxxcess/argos-osint/commit/4d44872d5df1e7d008cd54bf2c16da81f2335c49))
* **brain:** index Atlas actors in phase 6 and add global typed Related ([1bcd58c](https://github.com/xxxcess/argos-osint/commit/1bcd58c7c3208a4347ab14a41429020de27f3ae2))
* **brain:** reconcile actor sources and deduplicate identities ([8404f4b](https://github.com/xxxcess/argos-osint/commit/8404f4b5aeeccc8f3fa93fe46c5743af0c70e045))
* brief performance and a unified Brain memory graph ([143d504](https://github.com/xxxcess/argos-osint/commit/143d504a048230e3e1aacd1006cc1e8429124a69))
* **brief:** shared sentence index, block-aware cleanup and operation routing ([732922c](https://github.com/xxxcess/argos-osint/commit/732922c678f1dfe70eb4ae6fc82fdb5166fa5d74))
* **config:** role-reference overrides, portable schema 2, migration 41 ([764d06c](https://github.com/xxxcess/argos-osint/commit/764d06c68e79907dc904be9b344a98521eca1d89))
* **core:** request policy, brief stage timing and typed Brain kinds ([902ad1a](https://github.com/xxxcess/argos-osint/commit/902ad1a4fd74a862a10fca405a2383065542bb05))
* **core:** schema 43 drops the unused graph explanation table ([03b5bdf](https://github.com/xxxcess/argos-osint/commit/03b5bdf7057d5c94e3034fb8980912d9a995052f))
* **insights:** supersession as intentional retirement with linked reasons (§7, §8 cases 5–8) ([544ba74](https://github.com/xxxcess/argos-osint/commit/544ba74573c253ede866103222184656f445665c))
* **intel,brain:** generate concurrent deduplicated inferences ([339eca5](https://github.com/xxxcess/argos-osint/commit/339eca5a15f93e8267317eb95157750591946779))
* **intel:** add on-demand article and report insights ([79d9dd2](https://github.com/xxxcess/argos-osint/commit/79d9dd2ad0c6340ebdef83faa86a6cc6b059b116))
* **intel:** analyze claim context and publish each brief stage ([1d3eb59](https://github.com/xxxcess/argos-osint/commit/1d3eb59c8dae8e777fd0f9d36d495239017c8a42))
* **intel:** checkpoint per-claim inferences and complete with warnings instead of failing ([dee7391](https://github.com/xxxcess/argos-osint/commit/dee7391930c82eff2a024db7d3b81d5199d8a663))
* **intel:** intel-brief-v2 dependency graph, two-slot drainer, StageContext ([64901b2](https://github.com/xxxcess/argos-osint/commit/64901b218078e29c0c6bfd77aaff8f6331b18199))
* **intel:** per-artifact streamed progress in Background jobs; cente… ([bb0dc3a](https://github.com/xxxcess/argos-osint/commit/bb0dc3a857f0afc300d0ed1e0170a52ede1b73c7))
* **intel:** per-artifact streamed progress in Background jobs; centered spinners in panes ([b977b11](https://github.com/xxxcess/argos-osint/commit/b977b111f1fd9f6e280f31088a8f8e0a79cd70db))
* **intel:** send a per-model thinking effort for Inferences and size its deadlines for thinking ([592385b](https://github.com/xxxcess/argos-osint/commit/592385b316b558734c9fc7b73384882af1beab1e))
* **intel:** sequential Intel brief preparation and extraction ([a12bc9b](https://github.com/xxxcess/argos-osint/commit/a12bc9b05ff314b15e89d0432489c130628bbecb))
* **intel:** show failure, Providers and Retry hints in brief stage panes ([092f6c4](https://github.com/xxxcess/argos-osint/commit/092f6c4a50fda9de4d22cc853bc80a644ee936eb))
* **investigation:** assess retained source evidence before synthesis ([e0bfe14](https://github.com/xxxcess/argos-osint/commit/e0bfe14d388fe544ffa1cbb7ca978ff7260e2004))
* **jobs:** Jobs retry resumes brief chains; update diagrams, docs and graph ([da99619](https://github.com/xxxcess/argos-osint/commit/da9961977a2fa96367e828abb81e4318ca4e6605))
* model routing, streamed insights and reliable Brain publication ([406c74c](https://github.com/xxxcess/argos-osint/commit/406c74c142ba253444f8055f79425332feb664b3))
* **nlp:** preprocess source evidence with cached local NLP ([4173e44](https://github.com/xxxcess/argos-osint/commit/4173e44a0fc97b20e31b50a7abf678c44772bbad))
* **opencode:** replace legacy agent workflow with architect coder tester ([99beef7](https://github.com/xxxcess/argos-osint/commit/99beef7fc33f907f14aba3a79e53d85f43bcd6cb))
* **profile:** add sparkline and gauge visuals to KPI tiles ([45f5a3e](https://github.com/xxxcess/argos-osint/commit/45f5a3e0ec4c72a001c8dfb418d8199be93b2de2))
* **provider:** exact Jev capability routing and streamed insight timeout policy ([bcd49b6](https://github.com/xxxcess/argos-osint/commit/bcd49b6d4259a535c6078348ab9b2f5570d2b848))
* **providers:** add provider adapters and resolved capabilities ([7e72832](https://github.com/xxxcess/argos-osint/commit/7e7283273f46ebb6f14c1e30bbdf06a4615ef622))
* **providers:** bounded recovery, canonical Jev decisions, stage budgets ([f1d30ca](https://github.com/xxxcess/argos-osint/commit/f1d30caed6b1f7b3d3ee117ed8b4c8bab78fef84))
* **providers:** model role routing, provider adapters and parallel brief preparation ([13c635b](https://github.com/xxxcess/argos-osint/commit/13c635bd5377eae836ec22b494619e7d6ba68254))
* **providers:** remove the Grok (xAI) provider; old configs load and import with Grok routes dropped and a warning ([c3db481](https://github.com/xxxcess/argos-osint/commit/c3db481b85032ef97550d3c9b104df424426c1c7))
* **routing:** align callers and roles with operation-format capability selection ([f35c5e1](https://github.com/xxxcess/argos-osint/commit/f35c5e107e24c510ace6cb4048c9a16c4f8ce34a))
* **storage:** migrate to disk-backed SurrealDB ([#53](https://github.com/xxxcess/argos-osint/issues/53)) ([c010ebe](https://github.com/xxxcess/argos-osint/commit/c010ebe638432c12280cce7b1f67eef853ccf258))
* **tasks:** atomic lease recovery, dependent task chains and Intel brief preparation manifest (spec §5, phase 1) ([e430228](https://github.com/xxxcess/argos-osint/commit/e430228db4dbc6324b08f4f16de6b3190323e8be))
* **topology:** deterministic scope summary, scope key, graph version, digest and saved-summary table (schema 43) ([8af7922](https://github.com/xxxcess/argos-osint/commit/8af792239f404fe579f12cc74d12a23898588d3a))
* **topology:** neighborhood brief builder, typed model packet, validator, cache/executor ([d1c11c0](https://github.com/xxxcess/argos-osint/commit/d1c11c03e00050379b3ac376eced1d3720dbb823))
* **topology:** neighborhood summary with generated Summarization upgrade ([a247160](https://github.com/xxxcess/argos-osint/commit/a247160375c5dad1bff100f724d142995c1616cf))
* **topology:** role-aware summary flush dispatch, lease fencing, watcher, delayed-reply tests ([b55fcc1](https://github.com/xxxcess/argos-osint/commit/b55fcc167a90480db2815023cc360f7608d37b64))
* **topology:** scope summary failure card, e2e worker test, docs, diagram, checklist ([a968a7b](https://github.com/xxxcess/argos-osint/commit/a968a7b53e58887aba4694d15f7cdc25646887e9))
* **topology:** scope summary panel, Summary page, worker and scrolling ([7abc171](https://github.com/xxxcess/argos-osint/commit/7abc17182bd7353d59cfaddb3bfa4139f3642e80))
* **topology:** Summarization-role scope narrative with freshness planning ([f2a0a5a](https://github.com/xxxcess/argos-osint/commit/f2a0a5aad5dc4be4fe0e9d4612d65eeeacc9866b))
* **tui,recon:** add job controls and gate Brain publication ([12ea5cb](https://github.com/xxxcess/argos-osint/commit/12ea5cbebd7a749d24cc8ba3b008cfe2a9881e2e))
* **tui:** center panel loading indicators via anim::centered_lines/draw_centered ([e235f4c](https://github.com/xxxcess/argos-osint/commit/e235f4c29b2183501745de9fbc981258f62b1e39))
* **tui:** dashboard KPI visuals, refreshed app descriptions, Recon tips ([322c528](https://github.com/xxxcess/argos-osint/commit/322c52842110f360ecd889b31a126f36e1ad76c3))
* **tui:** open memories directly in the topology mapper at one hop ([93479b3](https://github.com/xxxcess/argos-osint/commit/93479b3d148c403f4201c90b97bd1b56eb2b2981))
* **tui:** open memories directly in the topology mapper, remove the claim path graph view ([7569da1](https://github.com/xxxcess/argos-osint/commit/7569da11e64dc86bc5b215fc6b115585686a6ca5))
* **tui:** place the intel brief article image beside the brief content ([bc57de6](https://github.com/xxxcess/argos-osint/commit/bc57de6e09fd75fbf0a55691002c4d85f6252289))
* **tui:** refresh app descriptions and rotate Recon target tips on Home ([c075fcb](https://github.com/xxxcess/argos-osint/commit/c075fcb5cac56dd09eb67325f44ac50ae857c886))
* **tui:** shared loading animations across every loading state ([eaf3c5b](https://github.com/xxxcess/argos-osint/commit/eaf3c5b1b7d0920f55cabd3f87a27fbaf30f5960))
* **tui:** show ignored basis references from a generated neighborhood summary ([dab54bc](https://github.com/xxxcess/argos-osint/commit/dab54bc338259e2330f85c6b29ea2b379f8d5d65))
* **tui:** streamed insight status steps on Intel, Jobs and Logs ([46d5d52](https://github.com/xxxcess/argos-osint/commit/46d5d52fff46859207bedbf8a11ddba9221c73c5))


### Bug Fixes

* **atlas:** retry transient feed faults, follow same-host redirects, count quota limits separately and name acquisition failures in the run status ([0383638](https://github.com/xxxcess/argos-osint/commit/03836385928a003e505f98c95e7a6aed5d9a7e25))
* **brain:** bound extraction and preserve insight publication ([a72af82](https://github.com/xxxcess/argos-osint/commit/a72af82fde0f00c190eea2d99c746d20bc8a2d33))
* **intel,tui:** request-mode ladder, route circuit breaker, Atlas article test, Defaults popups ([66878bc](https://github.com/xxxcess/argos-osint/commit/66878bc9fe026257f76c5ad9acdddf2a649981bf))
* **intel:** brief Inferences latency, status truthfulness and graceful degradation ([c7984b8](https://github.com/xxxcess/argos-osint/commit/c7984b8812f48cf47622700a0204546d7dfa7ca9))
* **intel:** classify missing model deployments, stage-wide hard-reject memory, last-route overload retry, mode ladder on gateway 404; live Atlas test; popup screenshots ([5cd45cf](https://github.com/xxxcess/argos-osint/commit/5cd45cf8772de595a3e929f12934980af17636ba))
* **intel:** fall through to the next route on a stall and cap operation wall-clock time ([b301af7](https://github.com/xxxcess/argos-osint/commit/b301af76ffc884a8cb3011a0a6ac9e5fe0b6fe21))
* **intel:** keep brief failure hints compact so Links stays visible at 80x24 ([13890cf](https://github.com/xxxcess/argos-osint/commit/13890cfa29cfffb86527d6b2727d5328fd1e589f))
* **intel:** keep the real brief failure category and label attempts by task ([53fd7fa](https://github.com/xxxcess/argos-osint/commit/53fd7fafe0f9272efc76c1ab2a7e714bd596068f))
* **intel:** resume brief context and honor model effort ([d81684d](https://github.com/xxxcess/argos-osint/commit/d81684dff34b398233cb1af81cdc829e8ee85d18))
* **providers,brain,atlas:** pace requests and reconcile memory sources ([19321d1](https://github.com/xxxcess/argos-osint/commit/19321d1d9ef2b2d6fc4aafef6e8c47635819bb28))
* **scheduler:** redact local paths and key drainer error throttling by message ([fbc20cc](https://github.com/xxxcess/argos-osint/commit/fbc20cccdf393e3e2099e9b3c24b7d01bbe1e7cb))
* **scheduler:** surface drainer errors instead of discarding them ([609b089](https://github.com/xxxcess/argos-osint/commit/609b08931882b6cd0bb299df2f3cc51541c0b70d))
* stop ineligible queued heads from starving Intel brief stages ([462f48c](https://github.com/xxxcess/argos-osint/commit/462f48cd72b7fb1884b6e69acd9d9f7c48f1ca7f))
* stop model-cited record ids from rejecting neighborhood summaries ([5700a1e](https://github.com/xxxcess/argos-osint/commit/5700a1e7478713d367017a7497dbeaafa662197d))
* stop model-cited record ids from rejecting neighborhood summaries ([9a3bfd3](https://github.com/xxxcess/argos-osint/commit/9a3bfd3a8a8618c67230aa8045ceb72bc25117e6))
* **tasks:** apply claim eligibility in the selector so ineligible heads cannot starve brief stages ([4dbec40](https://github.com/xxxcess/argos-osint/commit/4dbec40f530b92da56727736d329260be41bd37f))
* **test:** stabilize flaky tests on main ([#54](https://github.com/xxxcess/argos-osint/issues/54)) ([a987513](https://github.com/xxxcess/argos-osint/commit/a987513c127836efb57bb14e684fdc7386dd3ccc))
* **topology:** bound the summary watcher and make lease renewal load-tolerant ([8920477](https://github.com/xxxcess/argos-osint/commit/892047755e0997f33dc210d558598c9ca2d12384))
* **topology:** time the lease heartbeat by elapsed time and trace stalls ([dba2fff](https://github.com/xxxcess/argos-osint/commit/dba2fff801d4129bd8445c472e9aeb08cd99beb8))
* **tui,brain:** keep database and index work off the event loop ([45b9316](https://github.com/xxxcess/argos-osint/commit/45b9316da7a3e631b13725c493bd4857cd71345d))
* **tui:** open provider and model popup cards for every Defaults role ([4b47e88](https://github.com/xxxcess/argos-osint/commit/4b47e88336b77f1d946da74af3e4773c28ce3f2f))
* **tui:** report unreadable or missing memories instead of defaulting to a claim topology ([9eeb2f2](https://github.com/xxxcess/argos-osint/commit/9eeb2f24ac0f322e9a4af48d4234060c0a51942b))
* **tui:** restore original confidence grade artwork ([ee9713e](https://github.com/xxxcess/argos-osint/commit/ee9713e9dd40844a935e7cb0f1d3f4dbd3b9ad2a))
* **tui:** show the offending key in the rejected-summary banner ([fc9814d](https://github.com/xxxcess/argos-osint/commit/fc9814d8be8b00e41edf079ae6142155b9659ef4))
* **tui:** singular '1 hop' and a current hint in the Brain list ([d51512b](https://github.com/xxxcess/argos-osint/commit/d51512bd3be11a74f8165757a4433e1483699c4b))
* **tui:** space list records and reconcile Intel body state ([882537f](https://github.com/xxxcess/argos-osint/commit/882537f8a08c5c0d1ab7870704d29697f86e6c92))
* **tui:** stop saving memory detail in the session and cover old session files ([414409e](https://github.com/xxxcess/argos-osint/commit/414409eb409354272b37c26844898e4e6c741614))


### Performance Improvements

* **devex:** remove redundant GSD and curate planning artifacts ([f5a00f4](https://github.com/xxxcess/argos-osint/commit/f5a00f49aea22d81c00689121964eea9fc56d0c5))
* **tasks:** add a partial index for the queued claim scan ([351288d](https://github.com/xxxcess/argos-osint/commit/351288d84a13cf0158b8d287e6b0d585e8e25b22))
* **topology:** build the Summary rows from the typed brief and cache them ([d692708](https://github.com/xxxcess/argos-osint/commit/d6927081dae1701ecda878804d91111955a9bfbf))
* **tui:** key the summary rows cache by a small struct and fix state diagram text ([cefb6dc](https://github.com/xxxcess/argos-osint/commit/cefb6dc7353788fe05aefca635814379aec517e2))

## Changelog

Release Please maintains entries for reviewed releases from Conventional Commits.
See [versioning and releases](docs/wiki/code/versioning.md) for compatibility and commit rules.

## Initial baseline

The workspace starts this release workflow at 0.1.0. This is a development
baseline, not a claim that a v0.1.0 tag or GitHub release has been published.
History before the configured bootstrap commit remains in Git history.
