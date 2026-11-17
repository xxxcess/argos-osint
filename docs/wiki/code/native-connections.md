# Native connections and registered context

[Client](../../../apps/fleetworks-desktop/README.md) · [Protected identity](native-workspaces.md) · [Control/design guide](native-design-system.md) · [Verification](../../../.planning/2026-11-12-native-pairing-implementation/acceptance.md)

[![Native pairing and authenticated reconnect](../../diagrams/native-connections.svg)](../../diagrams/native-connections.html)

## Cold-launch startup

Each cold launch of the production app plays the **Quiet Orbit** scene for 3 seconds over the same `#060709` canvas as setup and the services: a dim molten-glass sphere, an obsidian crescent and five obsidian-glass satellites on an inclined orbit. The complete scene is visible from the first frame and plays at its eight-second pace (it does not speed up to finish a turn). From 2.65 s it crossfades into **Choose your connection**, which offers **Local Embedded** and **Remote Gateway**; the choice takes input at 3.00 s. With Reduced Motion the scene is one still for the same 3 seconds. The orbit scales as one unit with the window (compact widths, fractional scale, high DPI). It is drawn from vector geometry, so there is no media file to fail; [how to rebuild and check it](../../design/native-workstation-handoff/brand-space-ui/fleetworks-splash-handoff.md#reproduce).

Choosing a destination only opens the existing setup for it. Nothing starts a companion, connects, or falls back to the other transport. Starting and connecting stay explicit actions there. The splash does not run again on reconnect.

A launch with `--api-origin` or `--api-token-file` skips the splash and choice, so scripted and CI launches are unchanged. Fixtures skip it too. The timeline is `apps/fleetworks-desktop/src/startup.rs` and the scene is `apps/fleetworks-desktop/src/orbit.rs`.

## Inbox and Assistant dock

The toolbar **Inbox** bell opens a popover with **All** and **Unread**, a list, and expandable detail. It lists only changes this app already observed: a visible job that stops being active, a readiness check that enters or leaves ready, not ready, degraded or unavailable, and the connection becoming ready or failing. Item text is fixed wording plus a job title or check name, never error text, URLs or credentials. It keeps at most 50 items, records one transition per job once, and clears when the connection authority changes. It sends nothing and makes no claim about device-wide alerts.

The contextual **Assistant** dock is built and tested but **disabled** (`Assistant::enabled` is false). It needs the typed chat capability, which the backend does not advertise yet, so sending would stay gated even if shown. While it is disabled the original context rail stays in place. When enabled it docks at 380 pt if the center view keeps at least 640 pt, stays hidden otherwise, and floats over the right edge only when the operator asks. Enter sends, Shift+Enter adds a line, and an input method that is composing keeps Enter. The draft stays with its pinned context, and a new connection authority clears both.

## Command panel

Cmd/Ctrl+K or the toolbar magnifier opens a grouped list of known actions: go to a page, Setup Gateway, Settings, Open Inbox, and Show or hide Assistant when it is enabled. Typing only filters that fixed list, and there is no arbitrary command runner. Arrow keys move, Enter runs, and Escape closes and returns focus to the toolbar button. Actions that could accept work, such as seed or index, only open their page, where the existing review step still applies. While the panel is open it is the only overlay.

<a id="pairing-and-keychain-agreed-target"></a>

## Pairing and Keychain

**Implemented flow:** Gateway URL → operator approval once → macOS Keychain or Linux D-Bus Secret Service (or explicit Linux session-only mode) → authenticated reconnect. [Verification status](../../../.planning/2026-11-12-native-pairing-implementation/acceptance.md) records actual checks; injected secret-store tests do not establish live Keychain/Secret Service or installed-app passes.

| Step | Behavior |
| --- | --- |
| Enter URL | Pin one validated origin: remote HTTPS or loopback HTTP. A new installation waits for Connect. A known selected pairing/access reference may reconnect on startup; an explicit URL restores only its own known reference. |
| Start / approve | Persist the exact request ID and pairing polling secret before HTTP (written to macOS Keychain, Linux Secret Service, or held in process memory when Linux session-only mode is active). The app displays request ID/code; an independently authenticated Operator approves it within the ten-minute challenge window. IDs/codes/installation markers alone grant no API access. |
| Save / connect | Authenticate the issued scoped Native credential, check compatibility, then save its secret in Keychain/Secret Service (or memory in Linux session-only mode) before reporting connected. Register/reuse the protected installation. Settings retain only origin/deployment/credential references (`system_keyring` or `session_only` storage mode) and recovery state. |
| Reconnect | Read the exact installation/origin keyring item (`system_keyring`) or live session memory, authenticate and verify the saved deployment. Never scan other items or fall back silently to files or unauthenticated access. Missing/locked Linux Secret Service requires either unlocking it and retrying or explicitly choosing session-only mode. |
| Recover | Lost HTTP outcomes and failed secret-store promotion retain the exact ID/secret/issuance intent. Retry replays that request; denied/expired/revoked access requires explicit Pair again. Keychain/Secret Service locked/denied/unavailable and network failures remain separate actionable states. |
| Stop / resume | Change gateway URL or leave setup to cancel/fence client callbacks. Focus loss pauses pending transport; focus return in setup resumes the retained request. Local cancellation does not revoke server access or cancel accepted jobs. |

### Pairing API

All routes are POST. `PairingClient` exposes only start/poll; protected APIs retain authenticated `Client` behavior. Both transports disable redirects/proxies and bound bodies/timeouts. Polling/access secrets never enter URLs, UI state, Debug or diagnostics.

| Route | Authority / result |
| --- | --- |
| `/api/v1/pairing/start` | Public bounded challenge request: request ID, installation marker, 256-bit polling secret; returns pairing ID/code/expiry/poll interval |
| `/api/v1/pairing/poll` | Exact pairing ID + polling secret; returns pending, denied, expired or approved. Approved contains scoped access; replay returns the same issuance, not a new credential |
| `/api/v1/admin/pairing/approve`, `/deny` | Operator-purpose credential with ReadBackend, RegisterWorkstation and ManageConfiguration; exact pairing ID/code. The server derives Native scope/epoch/grants |
| `/api/v1/admin/pairing/revoke` | Current Operator authority and credential ID; fences paired authority/admission and related registered/browser proof without rewriting accepted origins |

### Operator approval and revocation

Use the app's displayed request ID/code and a separately provisioned private **Operator** token file:

```sh
python3 scripts/fleetworks_pairing.py \
  --origin https://gateway.example.test \
  --operator-token-file /absolute/private/operator-token \
  approve --pairing-id "<displayed-id>" --approval-code "<displayed-code>"
```

Use `deny` with the same ID/code, or `revoke --credential-id <paired-id>`. The helper pins the origin, refuses redirects/proxies, bounds input/output and never prints bearer bytes. Native/WebRelay credentials cannot approve pairing. [Deployment credentials](../../../deploy/README.md#credentials)

An isolated macOS native review can use `python3 deploy/native_review.py up --approval-handoff --name <review>` to open a local operator prompt when Setup Gateway starts a pairing request. The prompt shows the origin, pairing ID and code so the operator can compare them with the native screen and explicitly click **Approve**; the native client never receives the Operator credential. The handoff is limited to the selected loopback review and expires within one hour. Existing Keychain access reconnects without a fresh approval. Standard launches and production approval continue through the separate Operator command above. [Review setup](../../../deploy/README.md#isolated-native-backend-review)

The [synthetic local pairing screen](https://github.com/xxxcess/argos-osint/pull/67#issuecomment-6880990486) records the waiting layout and retry control in the PR; the accompanying [CI artifact](https://github.com/xxxcess/argos-osint/actions/runs/43617239215) includes wide and compact PNG/tree evidence. These fixture images use a fake code and do not establish a live Keychain result.

Approval → authenticated readiness/capabilities → Keychain → connection. A gateway failure after approval preserves the pending request: correct backend configuration and choose **Retry**. HTTP 503 means setup is unavailable; it does not mean pairing is unsupported. [Deployment troubleshooting](../../../deploy/README.md#troubleshooting)

Revocation, changed issuer policy/scope/access epoch and reset invalidate paired authority. Local Keychain deletion alone is not server revocation. Existing static Native, WebRelay and headless credentials remain supported independently.

## Explicit operator/headless file mode

Normal GUI setup needs no token file. Explicit `--api-origin <origin> --api-token-file <private-file>` preserves the file-based launch override; it does not populate Keychain. Optional `--provision-gateway-credential` stores only the protected file-catalog binding and exits without GUI/network startup; it is not a prerequisite for normal pairing. Existing files are never silently adopted, migrated or deleted. [Commands](../../../apps/fleetworks-desktop/README.md#build-and-provision)

## Authority and registration

| Boundary | Contract |
| --- | --- |
| Credentials | Native, WebRelay, InternalService and Operator purposes have explicit permissions. Connect IDs and caller-supplied headers grant no authority. |
| Local access | Operator-approved origin-bound Keychain access for normal GUI; explicit file-based launch mode remains separate. Neither forwards another origin’s credential. |
| Identity | Connect/activity contract 2: opaque workstation ID = Connect ID; authenticated scope + protected installation marker determine identity. Optional IP never merges installations. New requests contain no workspace fields. |
| Transport | Pin origin; disable environment proxy/redirect forwarding; bound responses. Gateway admission/read boundaries revalidate current proof, permissions and revisions. |
| Invalidation | Gateway/access changes fence old clients, reads, caches and confirmations. Missing registration or optional services disables only dependent actions; permitted shared reads remain usable. |
| Retirement | Fence admission and sessions; drain associated jobs and uncertain effects, then retire the workstation. Retained v1 folders keep their protection; accepted causal work may finish while retiring. |

Schema 10 adds pairing and credential records. Coordinate bootstrap and compatible readers/writers using the existing deployment selection; preserve scope, installation/workstation identity, registration mappings, unresolved requests and immutable origins. The earlier workstation-v2 migration fences legacy registrations/sessions and requires fresh proof for those migrated entries; pairing does not replace retained static/headless credentials. [Registry migration](../../diagrams/connect-registry.html)

## Browser handoff

[![Central browser authority gate](../../diagrams/intel-connect-gate.svg)](../../diagrams/intel-connect-gate.html)

| Boundary | Contract |
| --- | --- |
| Connect | Enter an existing opaque workstation ID. Separately provisioned WebRelay resolves it; the browser cannot register guessed IDs. |
| Session | Host-only HttpOnly SameSite=Strict `fleetworks_connect_v2`; HTTPS adds Secure. Old workspace cookies require fresh Connect. Allowlist public fields and require Connect ID = workstation ID. |
| Cache/cursors | `workstation-v2` namespace includes deployment, workstation, registration and authority revisions/epochs; IP is not identity. Old workspace contexts/narrowing/cursors fail explicitly. |
| Protected access | Central server guard precedes page/RSC/prefetch, REST/export and live data. Host/Origin/ingress and expected context revisions fence forged requests/stale-tab mutations. |
| Lifecycle | Switch/expiry/denial clears Query/live state; logout revokes server proof. Bounded live rechecks include deployment/workstation/registration/revisions/epochs and close stale streams. Native may close independently. |

## Durable requests and activity

[![Immutable origin before scoped projections](../../diagrams/brief-job-context.svg)](../../diagrams/brief-job-context.html)

- Admission captures immutable creator context transactionally; reuse adds a genuine requester association. Causal jobs retain their accepted origin/lifecycle fence. Version-aware readers preserve historical child metadata without allowing new child admission.
- Membership precedes rows, totals, facets, paging, private details, results, logs, exports, streams and mutations. Shared status does not grant private access; unknown/system/unregistered work has no all-backend fallback.
- Uncertain delivery retains original operation/request ID, exact body/context and original registration. `GET /api/v1/workstation/requests/detail` resolves it read-only using `original_registration_id`. Unknown stays unresolved; navigation/reconnection never creates a replacement POST.

## Bounded operational logs

`@tag:jobs-persistence`

Jobs SQLite owns safe summaries, not provider stdout/payloads. Membership/private eligibility applies before querying that owner, including counts/facets/cursors. Native displays a bounded selected-job queue/log view; finite updates pause when hidden/unfocused and never cancel accepted jobs. No tabs or filter/follow/copy/export/retry/cancel toolbar.

Richer backend APIs retain independent permissions. Opaque cursors bind proof/scope/snapshot, expire and require restart after invalidation. Follow/export stay bounded; partial exports and unavailable counts remain explicit.

**Layout.** When the Jobs page is at least 940 pt wide, the paged list is a 300 pt column beside the selected job's detail. Below that the two stack, and below 660 pt the list and detail alternate with **Back to jobs**. Refreshes keep the selected job and page.

## Readiness and maintenance

CSV picker → local column check → authenticated gateway upload URL → streamed bytes → backend-validated dataset → confirmed actor/Natural job. No Mac path or local download server enters the handoff. Index build requires completed seeding with actors present; training creates a candidate without activation. Readiness and optional host metrics keep missing/stale observations distinct from success. [Maintenance figure](../../diagrams/workstation-readiness.html) · [Dataset workflow](../../../deploy/README.md#configured-native-maintenance-inputs)

Cycles use bounded workstation summaries and revision/idempotency-bound association tombstones. Delete preserves shared evidence/audit and does not cancel a cycle. Accepted receipts mean queued, not completed.

## Configuration and model choices

| Edit | Contract |
| --- | --- |
| Models | `/api/v1/models/defaults` advertises canonical roles and `configured_routes` choices, not exhaustive remote discovery. Select the actual provider/account/model tuple. Effort/variant share one supported provider/model setting. |
| Provider key | Preserve hidden account fields and unchanged secrets. Permissions/revisions still apply; conflicts retain pending edits. |
| Tool slots | Independent optional patches: omitted/null preserves; empty explicitly clears; nonempty replaces. Native blank fields preserve; Clear actions are explicit. Saved bytes are never returned. |

## Source and verification

[Contracts/SDK](../../../crates/fleetworks-contracts/src/) · [Native effects](../../../apps/fleetworks-desktop/src/) · [Protected settings](../../../crates/fleetworks-workstation/src/native/) · [Registry](../../../crates/argos-osint-core/src/storage/connect_registry.rs) · [Job context](../../../crates/argos-osint-core/src/storage/job_context.rs) · [Gateway boundary](../../../crates/argos-osint-api/src/context_boundary.rs)

The [acceptance tracker](../../../.planning/2026-11-12-native-pairing-implementation/acceptance.md) separates implementation, actual focused/gate results, inspected Runtime evidence, hosted platforms and installed-app checks. A source audit or generated image is not runtime approval.
