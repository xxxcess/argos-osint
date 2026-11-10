# Fleetworks

[Code route](index.md) · [Implementation plan](../../../.planning/2026-11-09-fleetworks-overhaul/task_plan.md)

Venture initializes a workspace, imports legacy credentials and model defaults on request, and launches the Fleetworks terminal clients. The clients use the Argos HTTP API. Only the API host opens application storage.

## Try the workspace and clients

From a checkout, build the launcher and six sibling executables.

```sh
cargo build --locked -p fleetworks-venture -p fleetworks-cli --bins
target/debug/venture --workspace ./review-workspace init
target/debug/venture --workspace ./review-workspace client profile workspace
```

Workspace selection uses `--workspace`, `FLEETWORKS_WORKSPACE`, then `.fleetworks` under the current directory. Venture displays the absolute workspace path. The initial directories are `datasets/`, `exports/`, `config/`, `secrets/`, `models/`, `logs/`, and `storage/`.

Use `config/workspace.toml` for `[datasets]` bindings and `[environment]` defaults. Dataset paths are relative to the workspace. Use `secrets/environment.toml` for private environment defaults. An explicitly set process variable takes precedence over either file.

For an explicit legacy import, run `venture --workspace ./review-workspace import --from "$HOME/.argos"`. Venture copies selected provider API-key configuration and model defaults to the destination workspace and reports only counts. A conflicting destination is an error. The source configuration and application data stay in the original Argos home.

## Connect to the API

The clients default to `http://127.0.0.1:8080`. Pass `--api-origin` or set `FLEETWORKS_API_ORIGIN` to select the API listener. The API bearer can come from `--api-token-file`, `FLEETWORKS_API_TOKEN`, `FLEETWORKS_API_TOKEN_FILE`, or the workspace's `secrets/api-token`. Use the same token as the API when it requires one. Unauthenticated HTTP is restricted to loopback. Remote connections require HTTPS and a bearer token.

See the [CLI command reference](../../../apps/fleetworks-cli/README.md) for route-specific commands and their API requirements. Use `venture status` for storage-backed readiness and `venture client <app> --help` for an individual CLI.

Compose orchestration, durable dispatch, separately runnable domain services, and terminal parity are tracked in the implementation plan.
