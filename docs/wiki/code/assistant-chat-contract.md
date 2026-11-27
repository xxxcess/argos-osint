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

The [V2 wire contract](../../../crates/fleetworks-contracts/src/chat/qa.rs) is available through the Embedded companion's inherited Unix socket. Distributed retains Inbox notifications. In Embedded, Workstation Settings shows Generator and Decision preferences, effective engines, provisioning, and context usage for a registered Embedded connection while Backend inference is on. Open the Assistant composer from the toolbar bell. Workstation Status keeps deterministic diagnostics accessible independently. The Inbox prompts the operator to enable inference when it is off. The composer requires a ready generator. The separate related-activity lookup remains available under its own capability and permissions. The transcript starts empty. Assistant reply text appears when a submitted question reaches the generator.

### Prompt flow

[![Native Inbox prompt flow from the composer to the Embedded companion and back](../../diagrams/inbox-prompt-flow.svg)](../../diagrams/inbox-prompt-flow.html)

1. The Rune composer sends the question, a request ID, the registered request context, and an optional source reference. The companion authorizes the request and rechecks the current registration. Evidence, resolved focus and conversation history come from the backend. A request-ID replay returns the original authorized turn. [Native Inbox](../../../apps/fleetworks-desktop/src/inbox_qa.rs) · [Embedded transport](native-connections.md#backend-types-and-parity)
2. Every new reply requires a ready generator. The embedded NLP worker has up to 100 ms, including admission and queue time, to identify question, subject, correction, negation and time hints. NLP Off, unsupported language, saturation or timeout use established question routing. Conversation parsing stays local and has its own result separate from article annotation and caches. [NLP service](../../../crates/argos-osint-core/src/nlp/mod.rs) · [Conversation resolution](../../../crates/argos-osint-api/src/inbox_qa/conversation.rs)
3. Admission freezes facts for the resolved subject. Current readiness, host metrics and job reads keep their current authorization and freshness requirements. Supported historical subjects may cite the Embedded observation journal, provided the caller can still read the resource. A historical citation keeps its original time and coverage, while admission records a separate retrieval time. A readiness range ending at the question's reference instant can also include a separately retrieved fresh reading. A running-jobs list includes the scoped count and up to two names the registration can read. [Evidence lookup](../../../crates/argos-osint-api/src/inbox_qa.rs) · [Observation journal](storage.md#embedded-workstation-observation-journal)
4. The leased executor publishes **Working**, selects the response plan during **Thinking**, and saves one accepted plan before **Writing**. The plan binds a response mode, ordered evidence and coverage, and any approved next step to the admitted snapshot. The planner renders one bounded instruction from that plan. Its sentence and any approved guidance carry no observation citation. A suggestion remains read-only. [Response planner](../../../crates/argos-osint-api/src/inbox_qa/decision.rs) · [Executor](../../../services/api-host/src/inbox_qa_service.rs)
5. The generator receives the original question. Workers with plan support also receive the compact resolved focus and instruction. The model selects operational wording that includes up to three exact authorized facts and any approved guidance. For typed clarification, a worker that negotiates decoding support produces reply tokens within the approved wording for that clarification. The backend and worker verify the reply, including passage coverage and timestamps for cited facts. A generator that cannot produce a verified reply leaves a terminal status. Legacy workers without typed clarification support can still generate supported responses. [Worker contract](../../../crates/fleetworks-contracts/src/qa_worker.rs) · [Decoder](../../../tools/assistant-bert/src/dialogue.rs)
6. Inbox reconciles the latest turn and ordered events by cursor. Assistant message bodies use the worker's draft or verified reply. Batched provisional text is labeled **Draft · unverified** until a terminal reply replaces it. The **Sources** icon on a reply opens a full-width list in the transcript area. Sources are grouped by type and show each cited value and its original observation time. Use **Back to reply** to return to the same exchange. Stop seals the current turn and preserves citations already published. Turn statuses remain visible when reply text is unavailable. Short processing stages can pass between UI updates. [Native transcript](../../../apps/fleetworks-desktop/src/inbox_qa.rs) · [Turn store](../../../crates/argos-osint-core/src/storage/inbox_qa.rs)

### Conversation and retention

Assistant answers general questions using the selected local model's knowledge. A question with a distinct workstation request and a general question can return a generated explanation alongside exact workstation observations the current registration may read. The model generates the general clause separately. The app displays the original local facts and their sources. For questions about visible Inbox notifications, the desktop supplies priority-ordered groups, severity and counts. The worker decodes the supplied values and the companion verifies them against the submitted summary. The summary comes from the desktop and carries no backend citation.

An attached source applies to one submission and stays on retries of that request. A standalone general question stays on the general answer path even if a client attaches a source. For a workstation question, an explicit selection or correction takes precedence over retained focus. A follow-up can reuse its authorized subject, accepted intent, time interval and constraints for a new evidence read. “What about now?” requests a fresh observation, and a comparable previous reading can appear as a separately timestamped citation. If a pronoun could refer to several subjects, Assistant asks for a choice and can apply the answer on the next turn. Calendar phrases such as “since yesterday” use the Embedded workstation’s local timezone. An ambiguous date can prompt a request for a specific interval. Pending choices survive cancelled and failed attempts until a verified reply supersedes them.

On companions advertising `inbox-qa-history-v1`, **Previous exchange** and **Next exchange** browse completed turns. An arrow hides at its corresponding end. A new submission returns the transcript to the current turn. Headers show the workstation's local time when available, a stopwatch with the elapsed response time, and the observed generator context usage. **Used previous exchange** indicates that a retained subject, choice or exchange informed the reply.

Focus, pending choices and accepted plans follow the turn store's 15-minute expiry and credential, registration, access epoch and history-generation fences. Reusable focus or choices are published with verified terminal replies. Recall can use up to two completed exchanges with a stored generator reply and valid usage, subject to the same scope and exclusions. For “What was your last answer?” the model evaluates the latest retained reply as the approved answer. Ordinary operational turns receive the question, typed focus and the selected evidence. Turning inference off cancels active work and clears retained conversations and plans. Observations in the separate journal expire independently. [Turn store](../../../crates/argos-osint-core/src/storage/inbox_qa.rs)

### Example workstation questions

| What you want to know | Ask |
| --- | --- |
| Active or queued jobs | “What jobs are running?” for a list, “Any jobs running?” for a count, or “How busy is the job queue?” |
| Retries and recent failures | “Any jobs waiting to retry?” or “Any failed jobs in the last 24 hours?” |
| CPU, memory, and disk | “Could you check the host resource snapshot?” or “How much RAM is the API host using?” |
| Service readiness | “Any blockers on this workstation?” or “What are the current unresolved status checks about?” |
| Actor catalog and index | “Does the catalog need seeding?” or “Is the actor index ready for use?” |
| A selected job or cycle | “How's this job doing?” or “Has this cycle finished?” |
| Logs for a selected job or cycle | “Any errors or warnings for this job?” |
| Active backend | “Which backend am I connected to?” |
| A fresh reading of the last supported subject | “And now?” |
| Recorded index activity | “What happened to the index since yesterday?” Local coverage determines which observations are available. |
| Checks mentioned in the previous reply | After “Status check?”, ask “Which ones are unresolved?” or “What are the 7 failed ones?” The reply uses the counts from the earlier summary to identify the group and checks its current state. |
| The previous exchange | “What was your last answer?” or “Recap the last exchange.” |
| General knowledge | “What is the capital of Japan?” |
| Workstation observation with an explanation | “What is the current memory usage, and what is RAM?” |
| Inbox notifications | “What are my most important notifications?” |

### Jobs and Inbox triage

Use **Find related activity** on a selected private job. The companion resolves the job in the current registration. An explicit stage/check relation can open the associated diagnostic. Missing relations, ambiguous choices and unavailable inference have distinct outcomes.

The inline lookup returns one compact result. In Inbox, **View status** on a readiness notification opens the current check. **Open Job Logs** on a job notification opens the selected job and its log view in **Jobs & Logs**. Navigation uses the ordinary destination authorization.

**Explain in Assistant** attaches the notification’s subject to the composer for the operator to submit. A readiness notice grounds the explanation in a fresh observation. Registration status comes from the caller’s verified registration. For a job notification, the companion reads recent sanitized entries for that job under the current registration, including jobs that are running, completed or failed. It scales the bounded tail read to the active model and passes up to two short excerpts alongside the required job status. The worker fits those excerpts and their prompt overhead within half its negotiated input window. The status still answers when authorized logs are absent, the worker lacks the capability, or no excerpt fits.

Workstation managers can choose the **Generator model** below the composer. This updates the same preference as Assistant settings and preserves the draft while the generator changes.

### Models and lifecycle

| Model | FP32 weights | Input tokens | Output tokens |
| --- | ---: | ---: | ---: |
| FLAN-T5 Base (preferred) | ~990 MB | 512 | 128 |
| FLAN-T5 Small (fallback) | ~308 MB | 512 | 128 |
| GPT-2 Small | ~548 MB | 896 | 128 |
| GPT-2 Medium | ~1.52 GB | 896 | 128 |

Both GPT-2 checkpoints share 1,024 positions across the prompt and reply. The current worker handshake requires the model-specific window. GPT-2 uses up to 896 prompt tokens and reserves 128 positions for the reply. The question, focus, instruction and whole passages fit within that tokenizer budget. The instruction is capped at 64 input tokens within that window. Output stays within 60 words and 1,024 bytes. GPT-2 evaluates attention across the whole prompt and projects only the last position onto the vocabulary to predict the next token. The meter shows peak input usage against each turn’s negotiated budget. A retained turn from an older worker can still show its original 512-token measurement.

The companion requests response-plan support at launch. A worker that returns the corresponding Ready variant can receive the plan fields. Legacy workers receive the existing request shape and may report a 512-token input window. Requests that require new plan capabilities report unavailability with an older worker. Older companions omit the optional turn stage, so the native client uses the prior status labels. [Worker handshake](../../../crates/fleetworks-contracts/src/qa_worker.rs) · [Turn transport](../../../crates/fleetworks-contracts/src/chat/qa.rs)

**Generator** defaults to **Automatic**. It selects FLAN-T5 Base when fresh memory headroom allows it and can fall back to FLAN-T5 Small when Base assets are unavailable. The desktop pickers offer Automatic, Google (FLAN-T5 Base) and GPT (GPT-2 Small). Existing FLAN-T5 Small and GPT-2 Medium preferences apply until the user chooses a different setting. Settings shows the saved preference and the effective model once a verified worker is ready. All models use the same resource admission and sandbox checks. The warm worker stays idle between turns and runs without network access. [Pack provisioning and resource limits](../../../deploy/inbox-qa.md)

Each control sends a revisioned patch for its own role. Only companions advertising `inbox-qa-model-settings-v1` receive the preference fields. Legacy enablement requests leave both preferences intact, and older companions retain their read-only model display. A generator change fences active output and retains completed exchanges. Usage is attributed to the worker model and generator generation that produced it. Changing Decision leaves the generator generation and its warm worker in place. Turning Backend inference off reaps workers and retains preferences for the next enable.

Within the Embedded companion, settings changes, worker publications, and health updates queue before opening their write transactions. This prevents background writes from repeatedly invalidating a settings transaction. The database still enforces the authorization and generation fences and clears retained exchanges atomically when inference is turned off. The executor retries worker shutdown and clears the replica record only after confirmed exit, including while inference is disabled.

Each exact host fact preserves its decimal value, unit, subject and observation time. Jobs evidence comes from the authorized workstation scope. Resolved, failed, unknown and retired checks remain distinct, and passed checks can cite the authenticated registration. Successful replies report model input usage. Model-generated text stays read-only. The zero-shot classifier has its own model and execution path.

### Decision model qualification

**Decision** offers **Rules only** (default), **Julia-1 Q8** (168 MB), and **vLLM d3-edge Q8** (806 MB). The latter two save a preference immediately. Until the exact checkpoint and runtime qualify on the companion’s native target, **In use** stays **Rules only** and the panel reports **Awaiting qualification on this platform**. Unqualified choices trigger no model download or worker launch. Explicit links continue to use the established lookup rules. All new Assistant replies still require a verified generator result.

The [hosted scoring workflow](../../../.github/workflows/fleetworks-decision-qualification.yml) accepts an exact PR SHA through `Fleetworks` dispatch with `decision_qualification=linux` or `macos`. It verifies pinned Julia-1 and vLLM d3-edge assets, calibrates on a separate incident group, and records held-out precision, coverage, out-of-scope choices and warm eight-candidate p95. Its receipts describe isolated scoring. Julia uses the publisher’s October 19 Q8 snapshot because the later snapshot has a tokenizer metadata incompatibility with the pinned shared runtime. [Julia GGUF loading issue](https://github.com/ggml-org/llama.cpp/issues/32882)

Release qualification requires combined answer and navigation checks, resource measurements with FLAN and indexing active, and separate evidence for each native target. Response planning and related-activity scoring have different task identities. Response planning starts with a complete rules plan. A scorer may select a bounded, prevalidated candidate only after qualification for the exact task, checkpoint and platform. An unavailable scorer, timeout, abstention or invalid choice retains the rules plan. The qualification table currently leaves both tasks on rules. A qualified decision request uses a complete bounded set of authorized job observations, reserves workstation memory and starts one verified CPU scorer on a private Unix socket. Only a validated menu choice may become a link, and the companion rechecks permission and the active decision revision before it returns. The scorer is reaped after the request. The Linux launcher confines its files and Unix listener with Landlock and seccomp. macOS activation also requires a verified native sandbox.

### Operations

| Private operation | Permission |
| --- | --- |
| `qa_settings` | Registered Native backend read |
| `qa_settings_update` | Registered Native `manage_inbox_qa`, current setting revision |
| `qa_turn_submit` | Registered Native interactive submit and the evidence source’s read grant |
| `qa_related_activity` | Registered Native backend read for readiness or workstation activity read for a job |
| `qa_latest`, `qa_events`, `qa_history` | Registered Native workstation activity read |
| `qa_cancel` | Registered Native interactive submit |

Settings can include the current provision, verification, model load, or activation stage. An unavailable reason is a fixed code such as `pack_missing`, `pack_invalid`, `sandbox_unsupported`, `worker_start_failed`, `insufficient_resources`, or `model_provision_failed`. The notification feed remains available during setup and inference changes.

The optional `stage` travels with the latest turn and its ordered events for cursor-gap recovery. A turn's `provisional_reply` carries its bounded accumulated draft. A `provisional_delta` is ordered with the other events and has no citations. The canonical terminal turn and its verified findings replace the draft. Terminal transitions clear provisional text, including previously retained draft event text. A recovered turn with a partly published reply is sealed before it can append a second draft. Each active registration can submit one turn at a time. Per-placement worker and lease limits keep the pending work bounded. [V2 wire contract](../../../crates/fleetworks-contracts/src/chat/qa.rs)
