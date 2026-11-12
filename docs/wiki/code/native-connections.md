# Native connections and registered context

[Fleetworks](fleetworks.md) · [Native app](../../../apps/fleetworks-desktop/README.md) · [Endpoint inventory](../../../.planning/2026-11-11-rune-macos-cutover/phase1-design.md)

The macOS app connects to an independently running gateway. Connecting never starts containers, services, Intel Web, catalog seeding or model downloads. Atlas, Brain, Tools, Models and Profile show resources in the selected backend. Workstation, Workspaces and Jobs & Logs show local identity and registered activity. Registration records request provenance; it does not create a private actor catalog, model configuration or provider account for a folder.

[![Explicit native connection and registered browser handoff](../../diagrams/native-connections.svg)](../../diagrams/native-connections.html)

## Authority and registration

The gateway loads a versioned private credential manifest with distinct Native, WebRelay, InternalService and Operator tokens. Purpose and explicit permissions govern operations. A Connect ID is a lookup value, not an authentication credential. The compatibility bearer constructor grants none of the new purpose permissions. Invalid explicitly configured manifests fail startup instead of falling back to a shared token.

The app reads an explicit private token file, pins one HTTP origin, disables environment proxies and redirects, and bounds responses. The gateway validates current proof again at admission. Private services receive reduced server-created delegation through their fixed authenticated clients; public forwarded context headers cannot create authority.

Workstation Add/Edit saves a name, canonical gateway origin and a protected token-file reference in the installation-bound `.native-connections.json` catalog. The catalog contains no token bytes. The reference is bound to that exact origin and owned file object; replacing the file requires explicit selection again. Unknown formats, unsafe permissions, stale catalog revisions and another installation's catalog fail closed. The catalog is independent of workspace identity and retirement journals.

Test authenticates and checks compatibility without installing a connected client, registering a workspace or changing backend configuration. Connect performs the connection flow explicitly. Selecting or editing a destination invalidates old requests and scoped data; unsaved backend changes require an explicit discard decision. Saved selection never connects automatically. Local loopback and remote authenticated HTTPS profiles are both connect-only.

Native registration combines an explicitly selected current LAN/VPN address with one owned local child. The route to a remote gateway can suggest an interface; loopback does not select an external IP. The gateway enforces active IP, child-name and Connect-binding uniqueness transactionally. Installation/folder markers detect collisions but do not authorize claiming another workstation. Registration responses contain deployment, workstation, workspace and registration IDs plus a revision.

Before sending ensure, the local manager persists its exact registration attempt. An uncertain response keeps that attempt for exact retry and prevents removal until reconciled. Global reads can work before a folder is registered. Activity and mutations require the actual registered context and appropriate permissions. Configuration mutations carry an expected revision; a workspace change invalidates pending origin-dependent confirmations while keeping global drafts.

The [registry figure](../../diagrams/connect-registry.html) separates protected local identity from backend claims, active bindings, rebind and session revocation.

## Browser handoff

The gateway may advertise an explicit Intel Web URL. Native Open Intel Web opens its Connect form and copies the canonical Connect ID. The browser server uses its separately provisioned WebRelay credential to resolve an existing registration and sets an HttpOnly session. It cannot register a guessed or unknown ID.

The central web gate protects page loads, RSC/prefetch, API/export and live access. Session/context revisions fence old tabs and outstanding requests. Reconnection clears scoped caches and transport state; logout revokes server proof. IP/name lookup is not a fallback for an absent session.

[![Intel Connect authority and access gate](../../diagrams/intel-connect-gate.svg)](../../diagrams/intel-connect-gate.html)

## Durable requests and activity

Atlas, manual Tools, brief and maintenance admission preserve immutable creator context with the actual job transaction. Reuse adds a genuine requester association without rewriting the creator. Causal children inherit origin through the storage boundary. Legacy and system work stay unattributed.

Jobs queries filter requester membership before page limits, counts and facets. Native activity can cover the verified workstation or narrow to an owned child; browser activity stays in its selected child. Shared jobs expose lifecycle status while private details, results, logs and control remain unavailable to an incompatible cohort. Each retry/cancel action independently checks its permission, current proof, ownership and supported state.

The additive `sort=newest` Jobs route filters status, app, operation, kind, exact private correlation, text and creation time before paging. It includes eligible attributed causal children. Creation time descending and ID ascending anchor a fixed 72-hour window; current membership and status are rechecked on every response. An opaque `page_cursor` identifies the current page, including the first page, and `next_cursor` identifies an available continuation. Tokens expire after two minutes and bind the credential proof, scope, filters and window. Expired or changed context requires an explicit restart. The retained no-sort route keeps its existing active-first root view.

`matched_total` and app-facet completeness describe the current filtered cohort. Dashboard status buckets retain the separately labelled unfiltered top-level workstation scope. Neither value promises an immutable historical snapshot. Native paging keeps bounded cursor history without caching old page rows; applying filters clears stale selection and action confirmations. Jobs and Logs have separate tabs and drafts.

An uncertain submission retains its exact nonce and input/configuration snapshot. The read-only request lookup can recover a previously accepted association under current proof; a returned opaque job ID is verified through scoped Jobs before displaying results. Closing the app or cancelling an HTTP request does not cancel durable work.

[![Durable creator and requester context before activity projection](../../diagrams/brief-job-context.svg)](../../diagrams/brief-job-context.html)

## Bounded operational logs

`@tag:jobs-persistence`

The registered `/api/v1/logs`, `/logs/export` and `/logs/follow` routes use the same durable activity membership policy before querying the private Jobs SQLite owner. Whole-workstation reads may narrow to a child or selected job and its eligible descendants. Unassigned, other-node, other-deployment and incompatible mixed-origin private rows cannot affect returned rows, counts, facets or cursors. Historical domain event storage is not a fallback.

An eligible cohort over 512 jobs returns an explicit unavailable state with a narrowing hint. A page contains at most 100 safe summaries; raw payloads and sequence numbers never reach clients. App facets probe beyond the 32-name display limit and report completeness separately from counts. Search matches only the returned summary, including its 1,000-character bound. SQLite reads have an actual interruptible scan deadline and restore connection settings after interruption.

Opaque cursors bind the current credential proof, registered scope, filters and snapshot. They expire after two minutes, are process-local, and require an explicit restart after expiry, failover, revocation or membership change. Follow waits at most four seconds per request and rechecks authority; it is not a global subscription. Export is paged NDJSON with a continuation header. A page with a continuation is a partial export, and unavailable counts are unknown rather than zero.

## Source and verification

Workstation observations are a separate read-only contract: database seed completion, reader-visible actor search, a trained Natural candidate, explicit activation and verified Rust-BERT inference are distinct facts. Bounded checks preserve partial, unknown, stale and unavailable states. Seed/train/reindex and activation each require an explicit attributed mutation and a real durable consumer; accepting a job does not mean it finished.

[![Read-only readiness and explicit durable maintenance](../../diagrams/workstation-readiness.svg)](../../diagrams/workstation-readiness.html)

- [Shared IDs/context and SDK](../../../crates/fleetworks-contracts/src/) own canonical parsing, permissions, DTOs and bounded origin-pinned transport.
- [Native connection effects](../../../apps/fleetworks-desktop/src/connection.rs), registration and submission modules own asynchronous UI transitions and stale-reply fences.
- [Saved profiles](../../../crates/fleetworks-workspace/src/native/connections.rs) own the protected local catalog; [operational logs](../../../crates/argos-osint-api/src/operational_logs.rs) join authority to the private owner and bounded cursors.
- [Registry](../../../crates/argos-osint-core/src/storage/connect_registry.rs), [job context](../../../crates/argos-osint-core/src/storage/job_context.rs), [activity storage](../../../crates/argos-osint-core/src/storage/jobs_view.rs) and [gateway boundary](../../../crates/argos-osint-api/src/context_boundary.rs) own durable authority and membership.
- Contract/native/workspace tests and isolated storage fixtures cover identities, replay, revocation, retirement and privacy. Repository Playwright covers the real Connect gate, stale tabs and logout. Fixture and source checks do not replace installed-app validation on both Mac architectures or real isolated backend integration.
