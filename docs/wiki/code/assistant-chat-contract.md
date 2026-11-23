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

The native Inbox uses the separate [V2 types](../../../crates/fleetworks-contracts/src/chat/qa.rs) when the selected backend advertises `inbox-qa-v2`. Embedded uses the companion’s framed IPC. Distributed uses the paired gateway, including loopback gateways. Settings, the latest exchange, and transcript access follow the current registration and selected backend. The transcript sits above the composer and follows new answers while the reader is at the bottom. Scrolling up preserves the reading position. Each notification group has its own mark-as-read button. In an expanded group, a dot identifies each unread item. Collapsing an open notification marks that item read.

### Prompt flow

[![Native Inbox prompt flow from the composer to the selected backend and back](../../diagrams/inbox-prompt-flow.svg)](../../diagrams/inbox-prompt-flow.html)

1. The Rune composer submits the prompt, a request ID, the current registration context and an optional selected source. Embedded sends `qa_turn_submit` through the inherited Unix socket to the bundled API companion. Distributed sends the registered request to the paired gateway over HTTP(S). Both handlers call the same admission logic. [Native Inbox](../../../apps/fleetworks-desktop/src/inbox_qa.rs) · [Shared client](../../../crates/fleetworks-contracts/src/client/inbox_qa.rs) · [Embedded transport](native-connections.md#backend-types-and-parity)
2. Admission checks the registration, permissions and inference state, then builds a fresh, bounded snapshot. Readiness probes provide service and data checks. The selected backend samples CPU, memory and disk on its own host. Jobs contributes aggregate counts for the registered workstation when its credential permits activity reads. A follow-up such as “And now?” carries forward the last cited topic in this registration and takes new observations. An exact request-ID replay returns its original exchange. [Admission and evidence](../../../crates/argos-osint-api/src/inbox_qa.rs)
3. The backend routes the prompt. It greets, acknowledges, offers examples and asks for context where the question needs a source. Specific status and metrics questions use complete cited observations. “What does this mean?” can use the currently selected readiness check, job or cycle. A workstation readiness summary stores its frozen passages and queues a durable Inbox QA job. [Intent routing](../../../crates/argos-osint-api/src/inbox_qa/question.rs) · [Turn and event store](../../../crates/argos-osint-core/src/storage/inbox_qa.rs)
4. The Inbox QA executor claims a queued summary. Embedded supervises its offline BERT child through private framed pipes. Distributed runs an executor behind the gateway and communicates with the offline model container through a private Unix socket. The model receives the question and authorized evidence passages. The executor verifies selected spans before publishing them and can cite complete fresh readiness facts if the worker completes without a usable selection. [Executor](../../../services/api-host/src/inbox_qa_service.rs) · [Worker placement and isolation](../../../deploy/inbox-qa.md)
5. The native app renders the canonical turn and polls for committed events by cursor while work is active. The latest turn and canonical event pages let it recover after a pause or missed update. Each answer exposes its observation timestamps through the collapsible **Sources** control. Stop seals the turn with any citations already published. [Native transcript](../../../apps/fleetworks-desktop/src/inbox_qa.rs) · [Event contract](../../../crates/fleetworks-contracts/src/chat/qa.rs)

### Inbox QA routes

| Method | Path | Permission |
| --- | --- | --- |
| GET | `/api/v2/workstation/chat/settings` | Registered Native backend read |
| POST | `/api/v2/workstation/chat/settings` | Registered Native `manage_inbox_qa`, current setting revision |
| POST | `/api/v2/workstation/chat/turns` | Registered Native interactive submit and backend read |
| GET | `/api/v2/workstation/chat/latest` | Registered Native workstation activity read |
| GET | `/api/v2/workstation/chat/events` | Registered Native workstation activity read |
| POST | `/api/v2/workstation/chat/cancel` | Registered Native interactive submit |

When Embedded inference is unavailable, settings may include an `availability_reason`. The reason is a fixed code such as `pack_missing`, `pack_invalid`, `sandbox_unsupported`, or `worker_start_failed`. It contains no paths or process output. An unavailable reason leaves notifications usable and hides the QA composer. Distributed availability comes from the selected gateway.

The optional `startup_stage` reports the current Embedded pack download, verification, model load, or activation. The native setup, Connection settings and Inbox display one changing label while the backend sends progress. Verification and activation remain subject to the companion's normal checks. Ready means the worker loaded the verified pack and the executor established its ready lease.

Known readiness questions cite the matching fresh check. The answer distinguishes a reported failure, a verification failure, an unknown state, and a check that does not apply. Classifier service readiness, Atlas Rust-BERT inference, and legacy Natural training have separate checks. An explanation requires a reason recorded by that check. Explicit workstation or backend readiness summaries stream model selections from current checks. The coordinator validates the selected spans and publishes complete, relevant observations with timestamps in citation metadata. When the model completes without a usable selection, the coordinator can cite complete fresh readiness observations from the same snapshot.

Each new accepted turn refreshes a bounded snapshot of readiness, host metrics, and permitted workstation Jobs counts. Use questions such as “How much memory are we using?”, “What is our resource usage?”, and “How are the jobs doing?” to get the relevant measurements. CPU, memory and disk describe the connected gateway host or the local Embedded companion host. A `host_metric` finding carries the exact value in tenths of a percent, unit, subject and original `observed_at`. A `job_count` finding carries the registered workstation’s authorized queued, running or retrying count. Each observation keeps its own timestamp. Resource and Jobs summaries can cite up to three measured facts. Missing or stale observations are omitted, and zero is a valid measurement.

Distributed clients request typed metric evidence with `X-Fleetworks-Inbox-Evidence: 1`. Without that opt-in, the gateway encodes metric citations with the legacy `readiness` kind and omits `measurement`. Both views preserve the same turn, cited sentence, observation time, event sequence and cursor across submit, replay, latest, events and Stop. HTTP responses vary by this header and are private and non-cacheable. The bundled Embedded client and companion use typed citations over IPC.

The local assistant welcomes ordinary conversation and uses workstation evidence for current factual answers. For a broader question it asks what the user is trying to do or which source to use. The optional `conversation` field distinguishes acknowledgements, help and context requests. The existing `notice` and turn states remain usable by older clients. The admission transaction persists grounded answers and their events together. The Sources disclosure starts collapsed and retains its state through updates to the same answer. Each citation shows the original observation time.

Published findings are checked against the frozen evidence and its exact half-open Unicode scalar offsets. The latest answer holds at most three findings and 60 words. Events use a per-turn cursor with a bounded wait and include the canonical turn for gap recovery. Stop preserves committed findings and seals the turn. A request ID replay returns the existing turn when the original input matches. Off fences further QA and retires the displayed exchange. See [deployment and pack installation](../../../deploy/inbox-qa.md).
