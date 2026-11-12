# Atlas dispatch with an embedded queue

Add `compose.atlas.yml` to the [standalone native-review base](README.md) to start the Atlas worker and separate warm NLP and classifier services. Jobs owns the disk-backed RocksDB queue and exposes authenticated private delivery operations. SurrealDB currently owns job/task revisions, execution leases and domain results; the queue carries wake-ups and retains dead letters. Only the gateway and explicitly selected Web overlay publish host ports. The [job persistence contract](../docs/wiki/code/jobs-persistence.md) distinguishes the implemented transport foundation from the planned SQLite scheduler cutover.

## Start directly with Compose

Provision a fresh isolated deployment using the [backend recipe](README.md#isolated-native-backend-review). The helper creates private service tokens, including the Atlas internal token; keep its credentials, scope/epoch and numeric owner UID/GID across restarts. Build the backend images, then explicitly select the Atlas overlay with the same env file and project:

```sh
docker compose --env-file "$REVIEW_WORKSPACE/compose.env" \
  -f deploy/compose.native-review.yml -f deploy/compose.atlas.yml \
  --project-name "$REVIEW_PROJECT" up --no-build --pull never --detach --wait
```

Add `-f deploy/compose.yml` only when a Web origin/port was explicitly provisioned and its image built. Use the same selected Compose files, env file and project for `ps`, `logs`, restart and `down`. Stop before changing topology. The native app never starts this deployment. No AMQP URL, RabbitMQ server or Erlang installation is needed.

Jobs persists SQLite logs and the RocksDB queue under `storage/jobs/`. Only Jobs opens those files. Atlas connects through `FLEETWORKS_JOBS_ORIGIN` and `FLEETWORKS_JOBS_TOKEN_FILE`; Compose supplies both. `FLEETWORKS_QUEUE_NAME` defaults to `fleetworks.atlas`. A successful enqueue is durable before the domain outbox is marked published. Delivery visibility leases, acknowledgments, requeues and dead letters are separate from the authoritative execution lease.

## Upgrade an existing RabbitMQ workspace

Drain existing Atlas jobs using the previous installation, then shut down its owned processes before starting the new deployment. Already-published Rabbit messages cannot be recovered merely by restarting the domain outbox relay. Startup rejects pending legacy dispatches instead of discarding them. Preserve old queue data, credentials and ownership evidence for explicit recovery; retired launchers are not current shutdown tools.

Switching back to the base stack disables new Atlas dispatch at bootstrap and preserves queued work. Stop the overlay first; reapply it to resume dispatch.

Once the real worker and required provider/model configuration are ready, submit a collection cycle from native Atlas. Its durable job/request receipt is the acceptance authority. After an uncertain response, reconcile the same request before attempting new work; use Jobs to inspect authorized progress. Neither a healthy HTTP listener nor a returned receipt proves successful inference.

Atlas sends metadata cache misses to the warm NLP service in bounded batches. Its classifier sends exact model pins and batches to the private Rust-BERT service. The services bind only to the Compose network.

A live cycle needs GNews and NewsData.io keys plus a compatible Rust-BERT worker and pinned model. The standard image hosts the service and loads operator-supplied assets. Put a worker built for the container's Linux architecture and the MNLI checkpoint under the workspace's `models/atlas-bert/`. If the worker uses shared LibTorch libraries, make them available under `models/atlas-bert/lib/`. Set the worker executable and model directory in `classifier/zero-shot.json` using absolute container paths. For example

```json
{
  "worker_path": "/workspace/models/atlas-bert/argos-atlas-bert",
  "model_dir": "/workspace/models/atlas-bert/model"
}
```

The classifier readiness endpoint reports transport availability and a separate model state. The first successful Atlas pin verifies the model assets. If a pin fails, the Atlas job reports the failure reason in Jobs.
