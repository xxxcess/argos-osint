# Atlas dispatch with an embedded queue

Add `compose.atlas.yml` to the [Distributed native-review base](README.md) to start the Atlas worker and separate warm NLP and classifier services. Jobs owns the disk-backed RocksDB queue and exposes authenticated private delivery operations. SurrealDB currently owns job/task revisions, execution leases and domain results; the queue carries wake-ups and retains dead letters. Only the gateway and explicitly selected Web overlay publish host ports. The [job persistence contract](../docs/wiki/code/jobs-persistence.md) distinguishes the implemented transport foundation from the planned SQLite scheduler cutover.

## Start directly with Compose

Provision a fresh isolated deployment using the [backend recipe](README.md#isolated-native-backend-review). The local recipe keeps editable settings in repo-root `compose.env`. The helper creates private service tokens, including the Atlas internal token; keep its credentials, scope/epoch and numeric owner UID/GID across restarts. Build the backend images, then explicitly select the Atlas overlay with the same env file and project:

```sh
docker compose --env-file compose.env \
  -f deploy/compose.native-review.yml -f deploy/compose.atlas.yml \
  --project-name "$REVIEW_PROJECT" up --no-build --pull never --detach --wait
```

Add `-f deploy/compose.yml` only when a Web origin/port was explicitly provisioned and its image built. Use the same selected Compose files, env file and project for `ps`, `logs`, restart and `down`. Stop before changing topology. The native app never starts this deployment.

Jobs persists SQLite logs and the RocksDB queue under `storage/jobs/`. Only Jobs opens those files. Atlas connects through `FLEETWORKS_JOBS_ORIGIN` and `FLEETWORKS_JOBS_TOKEN_FILE`; Compose supplies both. `FLEETWORKS_QUEUE_NAME` defaults to `fleetworks.atlas`. A successful enqueue is durable before the domain outbox is marked published. Delivery visibility leases, acknowledgments, requeues and dead letters are separate from the authoritative execution lease.

## Dispatch and restart

The deployment starts with new Jobs stores and no queue migration prerequisite. Keep those stores and their credentials across subsequent restarts so acknowledged deliveries and pending work remain recoverable.

Switching back to the base stack disables new Atlas dispatch at bootstrap and preserves queued work. Stop the overlay first; reapply it to resume dispatch.

Submit news cycles through the registered Distributed API at `POST /api/v1/atlas/cycles`. In the desktop app, use **Tool Box** to configure provider keys and **Workstation Status** to inspect reported classifier readiness. The durable job receipt confirms queue acceptance. Use **Jobs & Logs** to see jobs associated with this workstation and any recorded failure. After an uncertain response, reconcile the same request before attempting new work.

Atlas sends metadata cache misses to the warm NLP service in bounded batches. Its classifier sends exact model pins and batches to the private Rust-BERT service. The services bind only to the Compose network. The overlay also configures the Distributed API to probe them with its private internal credential. Workstation Status can then report the selected backend’s observed NLP and classifier readiness. The API receives the private origins and token file paths through Compose. The native app receives the typed readiness observations. Embedded probes its own bundled engines through IPC.

A fresh cycle needs a saved primary or fallback key for each provider. GNews and NewsData.io supply discovery. NewsAPI and Currents supply collection. The worker checks all four slots before its first provider fetch. A saved key indicates presence only. Provider authentication is established when the provider accepts a request. Resumed cycles keep the requirements of their current phase. A cycle already in collection can resume with its collection keys.

Atlas also needs a compatible Rust-BERT worker and pinned model. The standard image hosts the service and loads operator-supplied assets. Put a worker built for the container's Linux architecture and the MNLI checkpoint under the workspace's `models/atlas-bert/`. If the worker uses shared LibTorch libraries, make them available under `models/atlas-bert/lib/`. Set the worker executable and model directory in `classifier/zero-shot.json` using absolute container paths.

```json
{
  "worker_path": "/workspace/models/atlas-bert/argos-atlas-bert",
  "model_dir": "/workspace/models/atlas-bert/model"
}
```

The classifier readiness endpoint reports service availability, checkpoint presence, and model verification. Checkpoint presence lets the first cycle proceed to its Atlas pin, which verifies the assets for the selected worker. If a pin fails, the Atlas job reports the failure reason in **Jobs & Logs**. The operator can evaluate and explicitly activate a frozen Atlas BERT candidate in the classifier workspace. [Qualification and activation](../docs/wiki/code/classifier.md#explicit-activation) · [Recalibration against reviewed provider mappings](../docs/wiki/code/classifier.md#recalibrate-on-reviewed-news-cycles)
