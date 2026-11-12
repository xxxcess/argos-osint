# Native connections and registered context

[Fleetworks](fleetworks.md) · [Native app](../../../apps/fleetworks-desktop/README.md) · [Endpoint inventory](../../../.planning/2026-11-11-rune-macos-cutover/phase1-design.md)

The macOS app connects to an independently running gateway. Connecting never starts containers, services, Intel Web, catalog seeding or model downloads. Atlas, Brain, Tools, Models and Profile show resources in the selected backend. Workstation, Workspaces and Jobs & Logs show local identity and registered activity. Registration records request provenance; it does not create a private actor catalog, model configuration or provider account for a folder.

## Authority and registration

The gateway loads a versioned private credential manifest with distinct Native, WebRelay, InternalService and Operator tokens. Purpose and explicit permissions govern operations. A Connect ID is a lookup value, not an authentication credential. The compatibility bearer constructor grants none of the new purpose permissions. Invalid explicitly configured manifests fail startup instead of falling back to a shared token.

The app reads an explicit private token file, pins one HTTP origin, disables environment proxies and redirects, and bounds responses. The gateway validates current proof again at admission. Private services receive reduced server-created delegation through their fixed authenticated clients; public forwarded context headers cannot create authority.

Native registration combines an explicitly selected current LAN/VPN address with one owned local child. The route to a remote gateway can suggest an interface; loopback does not select an external IP. The gateway enforces active IP, child-name and Connect-binding uniqueness transactionally. Installation/folder markers detect collisions but do not authorize claiming another workstation. Registration responses contain deployment, workstation, workspace and registration IDs plus a revision.

Before sending ensure, the local manager persists its exact registration attempt. An uncertain response keeps that attempt for exact retry and prevents removal until reconciled. Global reads can work before a folder is registered. Activity and mutations require the actual registered context and appropriate permissions. Configuration mutations carry an expected revision; a workspace change invalidates pending origin-dependent confirmations while keeping global drafts.

## Browser handoff

The gateway may advertise an explicit Intel Web URL. Native Open Intel Web opens its Connect form and copies the canonical Connect ID. The browser server uses its separately provisioned WebRelay credential to resolve an existing registration and sets an HttpOnly session. It cannot register a guessed or unknown ID.

The central web gate protects page loads, RSC/prefetch, API/export and live access. Session/context revisions fence old tabs and outstanding requests. Reconnection clears scoped caches and transport state; logout revokes server proof. IP/name lookup is not a fallback for an absent session.

## Durable requests and activity

Atlas, manual Tools, brief and maintenance admission preserve immutable creator context with the actual job transaction. Reuse adds a genuine requester association without rewriting the creator. Causal children inherit origin through the storage boundary. Legacy and system work stay unattributed.

Jobs queries filter requester membership before page limits, counts and facets. Native activity can cover the verified workstation or narrow to an owned child; browser activity stays in its selected child. Shared jobs expose lifecycle status while private details, results, logs and control remain unavailable to an incompatible cohort. Each retry/cancel action independently checks its permission, current proof, ownership and supported state.

An uncertain submission retains its exact nonce and input/configuration snapshot. The read-only request lookup can recover a previously accepted association under current proof; a returned opaque job ID is verified through scoped Jobs before displaying results. Closing the app or cancelling an HTTP request does not cancel durable work.

## Source and verification

- [Shared IDs/context and SDK](../../../crates/fleetworks-contracts/src/) own canonical parsing, permissions, DTOs and bounded origin-pinned transport.
- [Native connection effects](../../../apps/fleetworks-desktop/src/connection.rs), registration and submission modules own asynchronous UI transitions and stale-reply fences.
- [Registry](../../../crates/argos-osint-core/src/storage/connect_registry.rs), [job context](../../../crates/argos-osint-core/src/storage/job_context.rs), [activity storage](../../../crates/argos-osint-core/src/storage/jobs_view.rs) and [gateway boundary](../../../crates/argos-osint-api/src/context_boundary.rs) own durable authority and membership.
- Contract/native/workspace tests and isolated storage fixtures cover identities, replay, revocation, retirement and privacy. Repository Playwright covers the real Connect gate, stale tabs and logout. Fixture and source checks do not replace installed-app validation on both Mac architectures or real isolated backend integration.
