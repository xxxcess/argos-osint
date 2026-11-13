# Native connections and registered context

[Client](../../../apps/fleetworks-desktop/README.md) · [Protected identity](native-workspaces.md) · [Control/design guide](native-design-system.md) · [Verification](../../../.planning/2026-11-12-native-workstation-redesign/acceptance.md)

[![Current provisioning and agreed pairing flow](../../diagrams/native-connections.svg)](../../diagrams/native-connections.html)

## Pairing and Keychain (agreed target)

**Approved design; not implemented yet.** Native setup should be:

**Gateway URL → approve pairing once → macOS Keychain → connect.**

| Step | Target behavior |
| --- | --- |
| Enter URL | Validate and pin the exact gateway origin. Remote gateways require HTTPS; retain the existing loopback exception. |
| Approve pairing | A separately authenticated gateway operator approves a short-lived, single-use pairing request. A URL, installation marker or Connect ID alone grants no access. |
| Save access | Issue a revocable, scoped Native credential for this installation; store its secret only in macOS Keychain, bound to that origin. Settings retain non-secret references and recovery state. |
| Connect again | Read Keychain, authenticate, check compatibility and register/reuse the workstation. Routine reconnect needs no token entry or file selection. |
| Recover | Distinguish missing access, pairing denied/expired, Keychain locked/denied and gateway unreachable. Retry or re-pair explicitly; never downgrade to unauthenticated access or plaintext storage. |

Token files remain an operator/headless mechanism. Browser WebRelay credentials and HttpOnly sessions keep their independent boundary. Pairing must not reuse a browser Connect ID as a secret or forward an operator token into native settings.

Show one actionable setup state at a time. Before authentication succeeds, omit registration/readiness noise. Use plain messages such as **“Gateway access isn’t configured”** or **“Unlock Keychain to continue”**; keep provisioning flags out of the normal target flow.

**Implementation still required:** gateway pairing/credential issuance and revocation, a Keychain adapter, native progression/recovery and scoped integration tests. Exact endpoints, approval surface and lifetimes are not specified here. No pairing or Keychain runtime pass is implied.

## Current setup (transitional)

The current app still requires `--provision-gateway-credential --api-origin <origin> --api-token-file <private-file>` before its URL/check wizard. A missing/changed/unsafe binding fails before HTTP. Use the actual deployment origin: the documented local-review example uses `http://127.0.0.1:18091`; `8080` is a different binding. [Working provisioning instructions](../../../apps/fleetworks-desktop/README.md#build-and-provision)

## Authority and registration

| Boundary | Contract |
| --- | --- |
| Credentials | Native, WebRelay, InternalService and Operator purposes have explicit permissions. Connect IDs and caller-supplied headers grant no authority. |
| Current local setup | Exact-origin private-file reference, then URL/check; revalidate file identity/permissions. The agreed replacement above uses pairing and Keychain. Neither flow forwards another origin’s credential. |
| Identity | Connect/activity contract 2: opaque workstation ID = Connect ID; authenticated scope + protected installation marker determine identity. Optional IP never merges installations. New requests contain no workspace fields. |
| Transport | Pin origin; disable environment proxy/redirect forwarding; bound responses. Gateway admission/read boundaries revalidate current proof, permissions and revisions. |
| Invalidation | Gateway/access changes fence old clients, reads, caches and confirmations. Missing registration or optional services disables only dependent actions; permitted shared reads remain usable. |

Coordinate incompatible deployment: stop/fence old writers, run current operator bootstrap against retained data, then start compatible gateway/workers/clients. The atomic migration checkpoints contract/schema state, preserves workstation IDs and old registration mappings, marks legacy registrations read-only and revokes old browser sessions. Require fresh native/browser registration proof; preserve immutable origins and unresolved requests. [Registry migration](../../diagrams/connect-registry.html)

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

## Readiness and maintenance

Status distinguishes identity/gateway, service readiness and optional API-host metrics; missing/stale data is unavailable, never invented zero/success. Configured exact-revision inputs enable actor seed/index/Natural training. Missing/stale inputs disable only their action. Training creates a candidate without activation; Rust-BERT inference is separate. [Maintenance figure](../../diagrams/workstation-readiness.html) · [Operator inputs](../../../deploy/README.md#configured-native-maintenance-inputs)

Cycles use bounded workstation summaries and revision/idempotency-bound association tombstones. Delete preserves shared evidence/audit and does not cancel a cycle. Accepted receipts mean queued, not completed.

## Configuration and model choices

| Edit | Contract |
| --- | --- |
| Models | `/api/v1/models/defaults` advertises canonical roles and `configured_routes` choices, not exhaustive remote discovery. Select the actual provider/account/model tuple. Effort/variant share one supported provider/model setting. |
| Provider key | Preserve hidden account fields and unchanged secrets. Permissions/revisions still apply; conflicts retain pending edits. |
| Tool slots | Independent optional patches: omitted/null preserves; empty explicitly clears; nonempty replaces. Native blank fields preserve; Clear actions are explicit. Saved bytes are never returned. |

## Source and verification

[Contracts/SDK](../../../crates/fleetworks-contracts/src/) · [Native effects](../../../apps/fleetworks-desktop/src/) · [Protected settings](../../../crates/fleetworks-workstation/src/native/) · [Registry](../../../crates/argos-osint-core/src/storage/connect_registry.rs) · [Job context](../../../crates/argos-osint-core/src/storage/job_context.rs) · [Gateway boundary](../../../crates/argos-osint-api/src/context_boundary.rs)

The [acceptance tracker](../../../.planning/2026-11-12-native-workstation-redesign/acceptance.md) separates implementation, actual focused/gate results, inspected Runtime evidence, hosted platforms and installed-app checks. A source audit or generated image is not runtime approval.
