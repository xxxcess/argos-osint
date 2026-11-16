# Assistant chat contract

The typed wire contract for the native contextual Assistant lives in [`chat.rs`](../../../crates/fleetworks-contracts/src/chat.rs). It is **types, bounds and validation only**. No route, storage, job or transport implements it, and no backend advertises it. Clients keep sending gated until a backend advertises `chat` with `availability: supported` at the exact `CHAT_VERSION`, and the endpoints pass over both HTTP and IPC. The view parts are described in [native-design-system.md](native-design-system.md#assistant-conversation-parts-chat_uirs).

## Routes (proposed, not served)

| Method | Path | Needs |
| --- | --- | --- |
| POST | `/api/v1/workstation/chat/turns` | `SubmitInteractive` and a registration |
| GET | `/api/v1/workstation/chat/threads` | `ReadWorkstationActivity` and a registration |
| GET | `/api/v1/workstation/chat/messages` | `ReadWorkstationActivity` and a registration |
| GET | `/api/v1/workstation/chat/events` | `ReadWorkstationActivity` and a registration |

The server rechecks every permission. Advertising `chat` says what the backend implements, not what a caller may do.

## Capability

`Capabilities.chat` is absent when the backend does not implement chat. When present it carries a `version` and an `availability` of `supported`, `unsupported` or `unavailable`. These are kept apart from permissions and configuration so a client can say why sending is off. `ChatCapabilities::can_send` is true only for an exact version match that is `supported`.

## Turn

A turn request carries a durable `request_id`, the registered `context`, an optional `thread_id` with `expected_thread_revision`, a typed `source` (`actor`, `job`, `cycle` or `readiness` plus an ID) and the operator's `input`.

- The caller never sends evidence. The server resolves evidence for the source from its own authorized data. A request with an unknown field, including an `evidence` field, is rejected.
- A revision needs a thread ID.
- Input is at most 4 KiB, not empty, and holds no control character other than a newline or tab.
- The receipt returns the request, thread, turn, message and job IDs, the thread revision, the state and `reused`. A replayed request returns the same IDs with `reused` true and creates nothing new.

## Reads

| Read | Bounds |
| --- | --- |
| Messages | At most 50 per page. A page is at most 256 KiB. |
| Events | At most 100 per page. A read waits at most 4 seconds. Events are ordered by `sequence` and resume from `after`. |
| Final answer | At most 32 KiB. |
| Sources on a message | At most 16. Each is a kind, an ID and a short label, never evidence text. |

Turn states are `queued`, `running`, `retrying`, `final` and `failed`. V1 is a status, then one final answer. There is no partial text, no tool execution and no automatic publication.

## Not built yet

The server owns these, and they are not in this change: admission that checks current authority, reconciles the original request before the revision and active-turn checks, and atomically creates one turn, message, job and outbox entry. Also queue leases, resumable events, exact-once application of the final answer, the typed SDK client calls and the IPC operations.
