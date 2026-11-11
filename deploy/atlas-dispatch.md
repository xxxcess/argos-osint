# Atlas dispatch with an embedded queue

Add `compose.atlas.yml` to the [base stack](README.md) to start the Atlas worker and separate warm NLP and classifier services. Jobs owns the disk-backed RocksDB queue and exposes authenticated private delivery operations. SurrealDB currently owns job/task revisions, execution leases and domain results; the queue carries wake-ups and retains dead letters. The web and API remain the only host-published services. The [job persistence contract](../docs/wiki/code/jobs-persistence.md) distinguishes the implemented transport foundation from the planned SQLite scheduler cutover.

## Start with Venture

```sh
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" up --storage compose-disk --atlas
```

Venture records the overlay for status, logs, restarts and stop. Every storage mode uses the same embedded queue; `local-net` needs only a reachable SurrealDB and `local-process` starts only SurrealDB on the host. No AMQP URL, RabbitMQ server or Erlang installation is needed. Stop the owned stack before changing its configuration. [Storage settings](README.md#choose-a-storage-target)

## Start directly with Compose

Prepare the workspace and credentials in the [deployment guide](README.md#prepare-a-workspace). Keep the workspace owner’s numeric UID/GID so Jobs can write its data directory. Create the private NLP token:

```sh
mkdir -p "$FLEETWORKS_HOST_WORKSPACE/storage/jobs"
(
  umask 077
  set -C
  file="$FLEETWORKS_HOST_WORKSPACE/secrets/internal-token"
  [ -e "$file" ] || openssl rand -hex 32 >"$file"
)
docker compose -f deploy/compose.yml -f deploy/compose.atlas.yml \
  --profile disk up --build --detach --wait
```

For external SurrealDB, configure its URL/credentials first and omit `--profile disk`. Stop using the same Compose files, project name and profile with `down`.

Jobs persists SQLite logs and the RocksDB queue under `storage/jobs/`. Only Jobs opens those files. Atlas connects through `FLEETWORKS_JOBS_ORIGIN` and `FLEETWORKS_JOBS_TOKEN_FILE`; Compose supplies both. `FLEETWORKS_QUEUE_NAME` defaults to `fleetworks.atlas`. A successful enqueue is durable before the domain outbox is marked published. Delivery visibility leases, acknowledgments, requeues and dead letters are separate from the authoritative execution lease.

## Upgrade an existing RabbitMQ workspace

Drain existing Atlas jobs with the previous stack and then run `venture stop` before starting the new stack. Already-published Rabbit messages cannot be recovered by merely restarting the domain outbox relay. Startup rejects pending legacy dispatches until they drain. Old queue data and credentials are retained; Venture can still stop a legacy native broker supervisor. The old network-broker overlay is only a shutdown compatibility placeholder and is never selected for a new owner.

Switching back to the base stack disables new Atlas dispatch at bootstrap and preserves queued work. Stop the overlay first; reapply it to resume dispatch.

Once the worker is running, submit a cycle through Venture. Reusing the same request ID returns the original job and run IDs.

```sh
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" client atlas start --request-id review-001
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" client jobs detail fleet-atlas-job-review-001
```

Atlas sends metadata cache misses to the warm NLP service in bounded batches. Its classifier sends exact model pins and batches to the private Rust-BERT service. The services bind only to the Compose network.

A live cycle needs GNews and NewsData.io keys plus a compatible Rust-BERT worker and pinned model. The standard image hosts the service and loads operator-supplied assets. Put a worker built for the container's Linux architecture and the MNLI checkpoint under the workspace's `models/atlas-bert/`. If the worker uses shared LibTorch libraries, make them available under `models/atlas-bert/lib/`. Set the worker executable and model directory in `classifier/zero-shot.json` using absolute container paths. For example

```json
{
  "worker_path": "/workspace/models/atlas-bert/argos-atlas-bert",
  "model_dir": "/workspace/models/atlas-bert/model"
}
```

The classifier readiness endpoint reports transport availability and a separate model state. The first successful Atlas pin verifies the model assets. If a pin fails, the Atlas job reports the failure reason in Jobs.
