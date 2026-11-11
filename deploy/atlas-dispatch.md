# Atlas dispatch with RabbitMQ

Add `compose.atlas.yml` to the [base stack](README.md) to start the Atlas worker, its private RabbitMQ broker, and separate warm NLP and classifier services. The overlay enables the Atlas dispatch pool during database bootstrap. SurrealDB owns jobs, delivery revisions, and results. RabbitMQ carries durable wake-up messages and keeps a separate dead-letter queue. The web and API remain the only host-published services.

## Start with Venture

```sh
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" up --storage compose-disk --atlas
```

Venture records the overlay in the workspace's ownership file and reuses it for status, logs, restarts, and stop. With `--storage local-net`, Atlas uses the RabbitMQ URL in `FLEETWORKS_BROKER_URL_FILE`. With `--storage local-process`, Atlas uses the RabbitMQ process Venture starts on the host. The other storage modes use the bundled broker. Use `venture stop` before changing an owned workspace between the base and Atlas stacks. [Connection and startup settings](README.md#choose-a-storage-target)

## Start directly with Compose

Prepare the workspace and its storage credentials as described in the [deployment guide](README.md#prepare-a-workspace). Keep `FLEETWORKS_UID` and `FLEETWORKS_GID` set to the workspace owner so the broker can write its database and Erlang cookie. Create the broker data directory and owner-only credential files for RabbitMQ and the private NLP service.

```sh
mkdir -p "$FLEETWORKS_HOST_WORKSPACE/storage/rabbitmq"
(
  umask 077
  set -C
  for name in broker-password internal-token; do
    file="$FLEETWORKS_HOST_WORKSPACE/secrets/$name"
    [ -e "$file" ] || openssl rand -hex 32 >"$file" || exit
  done
)
docker compose -f deploy/compose.yml -f deploy/compose.atlas.yml \
  --profile disk up --build --detach --wait
```

For an external SurrealDB, configure its URL and credentials first and omit `--profile disk`. The Atlas overlay itself is the opt-in. To stop it, use the same Compose files, project name, and storage profile with `down`.

RabbitMQ persists its queue data and user state under `storage/rabbitmq/`. Reuse the same password file when restarting that broker. To provide a different file path, set `FLEETWORKS_BROKER_PASSWORD_FILE` to an absolute host path. Both containers read the same file. They write their derived config or endpoint into private temporary files at startup. The default broker user and virtual host are `fleetworks`, and the queue is `fleetworks.atlas`. Use `FLEETWORKS_BROKER_QUEUE` if an alternate queue is needed.

The worker runs `argos-api-host --role atlas-worker` with the same network storage settings as the background worker. Its wrapper sets `FLEETWORKS_BROKER_URL_FILE` to the generated endpoint file. When invoking that role directly, point the same variable at an owner-only file containing the authenticated AMQP endpoint.

After switching a workspace back to the base stack, bootstrap disables new Atlas dispatch and preserves queued work. Reapply the overlay to resume processing. Stop the overlay first so its broker and worker are removed before starting the base stack.

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
