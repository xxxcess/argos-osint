# Use Fleetworks

[Concept route](index.md) · [Native and service guide](../code/fleetworks.md) · [Wiki](../../index.md)

Build the native macOS app and independently provision its Linux backend. The app opens offline setup; connect explicitly to the deployed gateway and select a protected native child plus LAN/VPN address to register context. [Native quick start](../../../README.md#native-macos-app) · [Direct Docker deployment](../../../deploy/README.md)

[![Native connection and registration](../../diagrams/native-connections.svg)](../../diagrams/native-connections.html)

Venture, the six terminal clients and the legacy `argos` executable are retired. Existing terminal records remain [historical evidence](../../archive/cli-retirement.md), not operational instructions.

## Intel

A separately deployed Intel Web server opens at `/connect`. Enter an existing registered Connect ID from the native app; the browser receives its own protected server session. The ID selects context and is not a credential. Native may close while the browser and accepted backend work continue.

Open **Bulletin** to browse retained articles by day and domain, search, or sort by recency and temperature. Selecting an occurrence opens its **Focus Brief**. It attaches to existing brief work or starts a new generation for the current inputs. Progress and completed evidence update in place. **Reload** explicitly starts another generation.

The brief shows the source description, identified actors, claims, inferences and Related leads. Actor actions open dossiers. Claim and inference actions open their provenance in **Topology**, with its accessible list twin. Related articles suggest possible context through retained shared mentions. [Intel web contract](../code/intel-web.md)

## Atlas

Explicitly enable the separate Atlas Compose overlay and prepare provider accounts plus architecture-matching pinned Rust-BERT assets. The app does not provision them. Submit collection through native Atlas only when the backend advertises its durable worker and the caller has permission. [Atlas setup](../../../deploy/atlas-dispatch.md)

A durable receipt confirms acceptance; Jobs reports progress and completion. After an uncertain response, reconcile the exact prior request instead of inventing success or automatically submitting new work. Collection, actor indexing and reconciliation can settle separately. [Atlas and Related](atlas-classification-and-related-articles.md)

## Brain

Use native Brain for canonical actor discovery and authoritative dossiers. Selecting a result uses its returned canonical ID. An empty or unavailable search index never falls back to a global catalog scan. Intel Web owns topology navigation from dossiers and Focus Briefs; the native app has no topology renderer. [Actor discovery](../code/actor-search.md) · [Topology](topology-mapper.md)

## Jobs and Logs

The native **Jobs & Logs** page has two tabs. Jobs lists only current registered workstation activity, optionally narrowed to a child. Its newest paging includes genuinely persisted associated and causal work; a parent link alone grants no membership. Filtered observed match totals are separate from the unfiltered root-job dashboard. A cursor expiry or changed scope requires explicit restart.

Logs uses the same membership/private boundary before counts, pages and safe-summary exports. Choose whole workstation, selected child or a selected job with descendants. Follow runs only while visible and focused. Loaded-row copy and bounded export label incomplete results; they do not expose private raw payloads. Retry and cancel require separate permission and supported state, and leaving the page never cancels accepted work. [Connections and scoped activity](../code/native-connections.md)

## Profile

Native Profile manages shared backend provider accounts and portable configuration with confirmed revisions. Models manages shared role defaults. Tools lists supported operations and returns durable job receipts. Workstation readiness observes partial bounded status; seed, rebuild, Natural training and candidate activation are separate confirmed maintenance actions. Selecting another local child preserves shared backend actors, configuration and model state.

## State

The native container owns local child identities, protected saved connection references and retirement journals. The independent Docker deployment owns credentials, datasets, models, domain records, Jobs files and search indexes. Stop services through the same explicit Compose project; stopping preserves its persistent data. [Protected local workspaces](../code/native-workspaces.md) · [Storage](../code/storage.md)
