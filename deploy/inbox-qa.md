# Deploy Inbox QA

Inbox QA starts Off for every target. The native app exposes the setting during setup and under **Backend inference** in **Settings → Connection**. Embedded runs the worker under the local companion using private framed pipes. Distributed runs the worker behind the selected gateway and needs a provisioned executor and model pair. [Prompt flow](../docs/wiki/code/assistant-chat-contract.md#prompt-flow) · [QA API](../docs/wiki/code/assistant-chat-contract.md#inbox-qa-v2)

## Embedded packs

Use the offline pack and digest record for the app’s architecture. Obtain the digests from the trusted release or the exact CI artifact used for verification. The pack importer verifies the archive, manifest, native architecture, and pinned model assets before selecting the new pack.

```sh
python3 scripts/fleetworks_qa_pack.py install \
  --target "$TARGET" --archive "$QA_ARCHIVE" \
  --archive-sha256 "$QA_ARCHIVE_SHA256" --manifest-sha256 "$QA_MANIFEST_SHA256" \
  --output "$PRIVATE_PACK_PARENT/$PACK_VERSION" --state-dir "$EMBEDDED_STATE_DIR"
```

Use the Embedded companion’s state directory for the selected installation. The pack parent and the state directory must be owned by the current user. The state directory is private. Enable inference after import. To replace a pack, turn inference Off first, import into a new version directory, and then turn it on.

Quick Start reports the active artifact download and the installer reports verification. The companion reports model loading and activation when inference starts. These stages appear in **Backend inference** and Inbox. Download and verification progress comes from the active install in that private profile. The status clears when the installer exits.

The Embedded worker requires Linux Landlock ABI 3 or newer, or macOS Seatbelt. The companion starts it with access to the verified pack and the system loader. If the host lacks the required sandbox, Inbox and Connection settings report that Embedded inference is unsupported on that host. Expand the details for the Landlock requirement. Missing packs, invalid packs, and worker startup failures have their own messages.

The app and companion are released independently of the optional model archive. New Chat Inference Service packs use CPU Torch 2.1.0, Rust-BERT 0.22.0, and pinned FLAN-T5 Small. Prepare dependencies on a connected build host. The release smoke runs with an empty model cache and disabled networking.

The companion requests context-window counts from compatible packs. Older packs continue to answer, and turns from those workers show an empty usage indicator when counts are unavailable.

The new pack carries FLAN-T5 Small under `model/generator`. The installer and companion verify the complete file group against pinned digests. The worker advertises generation after loading the model. Each generated turn uses a 256-token input budget and up to two recent exchanges from the current registration. Embedded and Distributed can still load verified legacy DistilBERT packs. The separately deployed zero-shot news classifier retains its own model and configuration.

FLAN-T5 Base, GPT-2 Small and GPT-2 Medium are candidates in the manual GitHub Actions model evaluation. Candidate measurements cover the locked model runtime, frozen examples, context processing, and the model process’s memory. Deployment eligibility also needs packaged transport and host resource verification.

## Distributed review deployment

Use the QA model image for the gateway’s Linux architecture. Prepare one empty socket directory with mode `0700` for the executor/model pair, owned by the isolated deployment UID. Select it with `FLEETWORKS_QA_SOCKET_DIR` and set `FLEETWORKS_QA_IMAGE` to the verified local image tag. Apply [`compose.inbox-qa.yml`](compose.inbox-qa.yml) after [`compose.native-review.yml`](compose.native-review.yml).

The executor uses the review deployment’s datastore and a dedicated credential for the `fleetworks.qa` Jobs queue. Provision a distinct random token in a private file and set `FLEETWORKS_QA_JOBS_TOKEN_HOST_FILE` to that file. The model container receives the shared socket directory, a read-only verified pack, and a small temporary filesystem. Its networking is disabled. Set a distinct `FLEETWORKS_QA_REPLICA_ID` per pair and retain it across restarts. A model listener serves one prediction slot. The supplied review overlay allocates one CPU and 1 GiB for the model container, plus half a CPU and 512 MiB for the executor. These are initial budgets for verification.

Grant `manage_inbox_qa` explicitly to a registered configuration manager. A pairing operator that has this explicit grant also passes it to the workstations it approves. Pairing operators can issue ordinary Native access without the QA management grant. Enable QA after the pair is deployed. Use the gateway lifecycle state to confirm readiness and unload.

Off fences new work immediately. The model listener reaps an unresponsive child when its 30-second session deadline expires. The gateway reports **Stopping** while a replica still owes unload confirmation. Retain the replica identity when recovering its executor so it can reconcile the prior model session.

The smoke test uses the exact preloaded runtime, datastore, search, and QA image tags. It creates and tears down its own Compose project, pairs against its gateway, and exercises the same contract as the packaged Embedded test.

```sh
python3 -B scripts/fleetworks_qa_smoke.py distributed \
  --name "$FRESH_REVIEW_NAME" --platform "$PLATFORM" --image "$FLEETWORKS_QA_IMAGE"
```
