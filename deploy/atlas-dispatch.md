# Atlas dispatch with managed RabbitMQ

Add `compose.atlas.yml` to the [base stack](README.md) to start the Atlas worker, its private RabbitMQ broker, and a warm Compromise NLP service. The overlay enables the Atlas dispatch pool during database bootstrap. SurrealDB owns jobs, delivery revisions, and results. RabbitMQ carries durable wake-up messages and keeps a separate dead-letter queue. The web and API remain the only host-published services.

## Start the overlay

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

Atlas sends metadata cache misses to the warm NLP service in bounded batches. A live cycle also needs configured news providers and a ready Rust-BERT classifier.
