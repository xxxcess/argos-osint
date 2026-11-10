# Fleetworks proposed design

Status: proposed; user review pending. Implementation is not authorized yet.

## Product and build outputs
Venture is the system entry point. Ship venture and fleetworks-brain, fleetworks-atlas, fleetworks-jobs (Jobs & Logs), fleetworks-tools, fleetworks-models and fleetworks-profile. Preserve React Native Web and Intel articles/briefs/actors/topology. Remove the existing Recon research pipeline after extracting shared dependencies.

## Service composition
| Service | Ownership |
| --- | --- |
| web | Next/RN Web and same-origin API/WebSocket relay |
| api | Versioned public HTTP routing, authentication and event stream |
| control | Workspace/Profile configuration, provider/model defaults, tool registry |
| jobs | Submission, dependencies, cancellation, retry, outbox dispatch and operational history |
| atlas | Feed collection, article lifecycle, cycles and derivative work |
| brain | Memories, actors, graph, recall and retained Intel briefs |
| nlp | Warm bounded batched annotation engine |
| classifier | Warm inference; training is a separate worker role using the same image |
| execution | Provider/model calls and global quotas/cooldowns |
| tools | Retained tool execution without Recon orchestration |
| rabbitmq | Durable work delivery |
| surrealdb | Optional disk-backed Compose storage profile |

Use versioned HTTP/JSON for commands/reads/batch inference, stream updates through API, and RabbitMQ for asynchronous jobs. Domain services own their records; cross-domain writes use contracts rather than arbitrary table mutation. Start with one SurrealDB deployment and scoped credentials. Keep API clients and shared terminal UI independent of service implementation libraries.

## Workspace and storage
Resolve --workspace, then FLEETWORKS_WORKSPACE, then cwd/.fleetworks. Create/gitignore the workspace; display the absolute host path in Venture/Profile. Mount at /workspace with least-required subdirectory access. Layout: datasets/, exports/, config/, secrets/, models/, logs/, storage/.
Dataset bindings and relative paths resolve against the workspace. Default Profile outputs/exports land there. Workspace environment configuration supplies secrets/settings; explicit process environment overrides it. Never log secret values or bake them into images.
Start fresh data; explicitly import only existing API-key configuration and model defaults, preserving source files. No automatic .argos database migration.
Storage selections: (1) Venture-managed host disk SurrealDB, (2) Venture-managed host memory SurrealDB, (3) external disk-backed SurrealDB, including the optional Compose service. Default full baseline uses Compose disk storage. No per-app embedded databases. Host service interfaces must be authenticated and reachable from containers; support Docker host-gateway routing. Remote endpoints support TLS. External disk persistence is an operator requirement, not inferred from a URL.

## Jobs and recovery
Use lapin with Tokio. SurrealDB retains authoritative job state. Commit dispatch outbox records transactionally; use publisher confirms, persistent messages, durable queues, bounded prefetch, manual acknowledgement, retry limits and dead letters. Messages contain job ID, revision, workspace generation and correlation IDs, not secrets or large documents.
Keep leases, dependency fencing, durable cancellation and idempotent completion. A completion-before-ack crash must be harmless. New in-memory DB generation rejects stale broker deliveries. Periodic Atlas scheduling retains single-owner election where needed. Preserve bounded engine queues and shared provider admission across worker replicas.

## Compose and lifecycle
Provide base stack, disk SurrealDB profile, dev override and example configuration. Publish web/API on loopback by default; internal services stay on the private network. Use health/readiness checks, retries, reconnection and graceful signals. Persist local DB and broker under workspace storage. Venture owns Docker access on the host; do not mount its socket in app containers.
Venture web mode offers Jobs & Logs TUI or streaming server logs. CLI clients inherit resolved workspace/endpoints and never open stores or start schedulers. Exiting a client leaves services running. Explicit stop only stops owned services. Distinguish host metrics from container metrics.
Build native Linux ARM64/AMD64 images with shared build stages and separately runnable service entrypoints. Compose topology here is a specification; runnable YAML comes during implementation after service commands/configuration exist.

## Organization and rollout
apps/ contains Venture, terminal clients and web; services/ contains backend entrypoints; crates/ and packages/ contain shared contracts/domain/UI libraries; deploy/ contains deployment assets. Extract seams before bulk moves/renames. Preserve functionality through each phase; remove obsolete embedded build graph only after server parity. No Kubernetes or remote app-stack management in v1.
