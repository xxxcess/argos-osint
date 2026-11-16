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

## Not built yet

The server owns these, and they are not in this change: assistant replies, storage of threads and messages, the three reads, and admission that checks current authority, reconciles the original request before the revision and active-turn checks, and atomically creates one turn, message, job and outbox entry. Also queue leases, resumable events, exact-once application of the final answer, the typed SDK client calls and the IPC operations.
