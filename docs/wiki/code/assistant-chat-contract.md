# Assistant chat contract

The typed wire contract for the native contextual Assistant lives in [`chat.rs`](../../../crates/fleetworks-contracts/src/chat.rs). V1 serves one endpoint, `POST turns`, over HTTP and over local IPC (`IpcOperation::ChatTurnSubmit`, `chat_turn_submit`). It checks the caller and the request and answers `{"status":"ok"}` (`ChatTurnAck`). It stores nothing, starts no job, and never writes an assistant reply. The backend advertises `chat` as `supported`, and the native composer clears its draft on send. On a failure the text comes back if nothing new was typed. The HTTP and IPC parity test checks the ok reply and a 400 for empty input on both transports. The view parts are described in [native-design-system.md](native-design-system.md#assistant-conversation-parts-chat_uirs).

## Routes

| Method | Path | Needs |
| --- | --- | --- |
| POST | `/api/v1/workstation/chat/turns` | Native credential, `SubmitInteractive` and a registration. **Served**: answers ok. |
| GET | `/api/v1/workstation/chat/threads` | `ReadWorkstationActivity` and a registration. Not served yet. |
| GET | `/api/v1/workstation/chat/messages` | `ReadWorkstationActivity` and a registration. Not served yet. |
| GET | `/api/v1/workstation/chat/events` | `ReadWorkstationActivity` and a registration. Not served yet. |

The server rechecks every permission. Advertising `chat` says what the backend implements, not what a caller may do.

## Capability

`Capabilities.chat` is absent when the backend does not implement chat. When present it carries a `version` and an `availability` of `supported`, `unsupported` or `unavailable`. These are kept apart from permissions and configuration so a client can say why sending is off. `ChatCapabilities::can_send` is true only for an exact version match that is `supported`.

## Turn

A turn request carries a durable `request_id`, the registered `context`, an optional `thread_id` with `expected_thread_revision`, a typed `source` (`actor`, `job`, `cycle` or `readiness` plus an ID) and the operator's `input`.

- The caller never sends evidence. The server resolves evidence for the source from its own authorized data. A request with an unknown field, including an `evidence` field, is rejected.
- A revision needs a thread ID.
- Input is at most 4 KiB, not empty, and holds no control character other than a newline or tab.
- The request's registration and context revisions must match the caller's current registration, or the reply is 409.
- The turn source must be a job, cycle or readiness check. The native app does not send from a plain page.
- V1 replies with `ChatTurnAck` only. `ChatTurnReceipt` (request, thread, turn, message and job IDs, revision, state, `reused`) is typed for the later durable version.

## Reads

| Read | Bounds |
| --- | --- |
| Messages | At most 50 per page. A page is at most 256 KiB. |
| Events | At most 100 per page. A read waits at most 4 seconds. Events are ordered by `sequence` and resume from `after`. |
| Final answer | At most 32 KiB. |
| Sources on a message | At most 16. Each is a kind, an ID and a short label, never evidence text. |

Turn states are `queued`, `running`, `retrying`, `final` and `failed`. V1 is a status, then one final answer. There is no partial text, no tool execution and no automatic publication.

## Inbox QA V2

The [V2 wire contract](../../../crates/fleetworks-contracts/src/chat/qa.rs) is available through the Embedded companion's inherited Unix socket. Distributed retains Inbox notifications. In Embedded, Workstation Settings shows Generator and Decision preferences, effective engines, provisioning, and context usage for a registered Embedded connection while Backend inference is on. Workstation Status provides an Assistant shortcut and keeps deterministic diagnostics accessible independently. The Inbox prompts the operator to enable inference when it is off. On a companion advertising `inbox-qa-triage-v1`, the composer can answer supported questions while model inference is starting or unavailable. An empty transcript shows an Assistant greeting once questions can be submitted.

### Prompt flow

[![Native Inbox prompt flow from the composer to the Embedded companion and back](../../diagrams/inbox-prompt-flow.svg)](../../diagrams/inbox-prompt-flow.html)

1. The Rune composer sends the question, a request ID, the registered request context, and an optional source reference. The backend uses the registration to authorize the request. It rejects caller-supplied evidence and conversation history. [Native Inbox](../../../apps/fleetworks-desktop/src/inbox_qa.rs) · [Embedded transport](native-connections.md#backend-types-and-parity)
2. Admission selects the readiness, host metrics, Jobs counts and selected activity needed for the question. It builds the model's snapshot from authorized readings. Recall requests can include up to two completed exchanges from the same credential, registration, access epoch, and generation. A follow-up such as “What about now?” reuses the authorized subject and supported intent when they are available, then collects fresh readings. A new explicit selection takes precedence. When the retained subject has a comparable earlier observation, the reply pairs the current reading with a separate timestamped citation for the previous one. Request-ID replays preserve the original turn and its authorization boundary. [Admission and evidence](../../../crates/argos-osint-api/src/inbox_qa.rs) · [Turn store](../../../crates/argos-osint-core/src/storage/inbox_qa.rs)
3. Exact single-fact questions have a direct path for typed timestamped facts, including the reporting window on failed-job counts. Supported paraphrases, follow-ups, and summaries with approved evidence use the ready generator first. The generator selects a bounded reply for operational questions, using approved wording and copying facts exactly from the snapshot. This preserves grouped readings and leaves room for shorter summaries with relevant details. When the worker cannot provide a verified reply, Assistant can return the authorized facts directly. The worker generates conversational replies token by token and checks for cancellation between decoding steps. [Intent routing](../../../crates/argos-osint-api/src/inbox_qa/question.rs) · [Executor](../../../services/api-host/src/inbox_qa_service.rs) · [Decoder](../../../tools/assistant-bert/src/dialogue.rs) · [Workstation question benchmark](../../../deploy/inbox-qa.md#workstation-question-benchmark)
4. Batched text updates arrive as provisional events. The operational prompt contains only passages referenced by its approved reply options. The worker and backend validate complete passages in operational drafts and final replies. Citations keep the passages' original observation times. When inference cannot produce a verified reply, the backend can commit the fallback findings and terminal state together. A grouped reading or comparison needs the complete fresh snapshot to answer. The terminal turn clears the provisional text, seals the canonical reply, and moves the observation boundary forward for the next turn. [Validation](../../../crates/fleetworks-contracts/src/qa_worker.rs) · [Event store](../../../crates/argos-osint-core/src/storage/inbox_qa.rs)
5. The transcript reconciles the latest turn and event pages by cursor. The compact dial reports the peak input count and percentage of the selected model's input window. **Sources** starts collapsed and shows each citation's observation time when expanded. Stop seals the turn with any already published citations. [Native transcript](../../../apps/fleetworks-desktop/src/inbox_qa.rs)

### Jobs and Inbox triage

Use **Find related activity** on a selected private job or expanded readiness notice. The companion resolves a typed job or readiness reference in the current registration. An explicit stage/check relation can open the associated diagnostic. A historical readiness notice links to **Current status**. A current check alone provides no evidence of the earlier cause. Missing relations, ambiguous choices and unavailable inference have distinct outcomes.

The inline lookup returns one compact result and creates no Assistant exchange or job. **Explain in Assistant** fills the composer with the selected subject for the operator to submit. Both flows use scoped activity and fresh readiness. Navigation is a typed native action that runs after operator input and uses the ordinary destination authorization.

### Models and lifecycle

| Model | FP32 weights | Input tokens | Output tokens |
| --- | ---: | ---: | ---: |
| FLAN-T5 Base (preferred) | ~990 MB | 512 | 128 |
| FLAN-T5 Small (fallback) | ~308 MB | 512 | 128 |

**Generator** defaults to **Automatic**. It selects Base when fresh memory headroom allows it and can fall back to Small when Base assets are unavailable. Explicit **FLAN-T5 Small** always requests Small. Explicit **FLAN-T5 Base** stays selected if memory or verified assets are unavailable. The panel keeps the saved choice visible and reports the effective model once a verified worker is ready. Both choices use the same resource admission and sandbox checks. The warm worker stays idle between turns, runs without network access, and uses one beam. [Pack provisioning and resource limits](../../../deploy/inbox-qa.md)

Each control sends a revisioned patch for its own role. Only companions advertising `inbox-qa-model-settings-v1` receive the preference fields. Legacy enablement requests leave both preferences intact, and older companions retain their read-only model display. A generator change fences active output and retains completed exchanges. Usage is attributed to the worker model and generator generation that produced it. Changing Decision leaves the generator generation and its warm worker in place. Turning Backend inference off reaps workers and retains preferences for the next enable.

Within the Embedded companion, settings changes, worker publications, and health updates queue before opening their write transactions. This prevents background writes from repeatedly invalidating a settings transaction. The database still enforces the authorization and generation fences and clears retained exchanges atomically when inference is turned off. The executor retries worker shutdown and clears the replica record only after confirmed exit, including while inference is disabled. An operational turn that reaches the executor with no inference time left goes directly to fallback and preserves the warm worker.

Each exact host fact preserves its decimal value, unit, subject and observation time. Jobs evidence comes from the authorized workstation scope. Resolved, failed, unknown and retired checks remain distinct, and passed checks can cite the authenticated registration. Direct responses report zero model input tokens. Older workers can omit usage or report their legacy limit. Model-generated text stays read-only. The zero-shot classifier has its own model and execution path.

### Decision model qualification

**Decision** offers **Rules only** (default), **Julia-1 Q8** (168 MB), and **vLLM d3-edge Q8** (806 MB). The latter two save a preference immediately. Until the exact checkpoint and runtime qualify on the companion’s native target, **In use** stays **Rules only** and the panel reports **Awaiting qualification on this platform**. Unqualified choices trigger no model download or worker launch. Exact facts and explicit links use the same deterministic path with any preference.

The [hosted scoring workflow](../../../.github/workflows/fleetworks-decision-qualification.yml) accepts an exact PR SHA through `Fleetworks` dispatch with `decision_qualification=linux` or `macos`. It verifies pinned Julia-1 and vLLM d3-edge assets, calibrates on a separate incident group, and records held-out precision, coverage, out-of-scope choices and warm eight-candidate p95. Its receipts describe isolated scoring. Julia uses the publisher’s October 19 Q8 snapshot because the later snapshot has a tokenizer metadata incompatibility with the pinned shared runtime. [Julia GGUF loading issue](https://github.com/ggml-org/llama.cpp/issues/32882)

Release qualification requires combined answer and navigation checks, resource measurements with FLAN and indexing active, and separate evidence for each native target. No candidate currently has a release record. A qualified decision request uses a complete bounded set of authorized job observations, reserves workstation memory and starts one verified CPU scorer on a private Unix socket. Only a validated menu choice may become a link, and the companion rechecks permission and the active decision revision before it returns. The scorer is reaped after the request. The Linux launcher confines its files and Unix listener with Landlock and seccomp. macOS activation also requires a verified native sandbox.

### Operations

| Private operation | Permission |
| --- | --- |
| `qa_settings` | Registered Native backend read |
| `qa_settings_update` | Registered Native `manage_inbox_qa`, current setting revision |
| `qa_turn_submit` | Registered Native interactive submit and the evidence source’s read grant |
| `qa_related_activity` | Registered Native backend read for readiness or workstation activity read for a job |
| `qa_latest`, `qa_events` | Registered Native workstation activity read |
| `qa_cancel` | Registered Native interactive submit |

Settings can include the current provision, verification, model load, or activation stage. An unavailable reason is a fixed code such as `pack_missing`, `pack_invalid`, `sandbox_unsupported`, `worker_start_failed`, `insufficient_resources`, or `model_provision_failed`. The notification feed remains available during setup and inference changes.

A turn's `provisional_reply` carries its bounded accumulated draft for cursor-gap recovery. A `provisional_delta` is ordered with the other events and has no citations. The canonical terminal turn and its verified findings replace the draft. Terminal transitions clear provisional text, including previously retained draft event text. A recovered turn with a partly published reply is sealed before it can append a second draft. Each active registration can submit one turn at a time. Per-placement worker and lease limits keep the pending work bounded. [V2 wire contract](../../../crates/fleetworks-contracts/src/chat/qa.rs)
