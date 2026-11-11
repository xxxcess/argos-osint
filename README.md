# Fleetworks

<p align="center">
  <img src="assets/brand/fleetworks-logo.jpg" alt="Fleetworks" width="280">
</p>

Fleetworks collects public-source intelligence, connects evidence to actors and articles, and prepares cited Focus Briefs. Venture manages the local services that power the Intel web desktop and six terminal clients.

[Walkthrough](docs/wiki/walkthrough.md) · [Setup and usage](docs/wiki/code/fleetworks.md) · [Deployment](deploy/README.md) · [CLI reference](apps/fleetworks-cli/README.md) · [Wiki](docs/index.md) · [Releases](docs/wiki/code/versioning.md)

## Quick start

Install Rust **1.95.0**, Docker, and Docker Compose **v2.20 or newer**. Run from the repository root. The workspace defaults build Venture and its network clients.

```sh
cargo build --locked -p fleetworks-venture -p fleetworks-cli --bins
./target/debug/venture --workspace ./.fleetworks up
./target/debug/venture --workspace ./.fleetworks status
```

Open the web address printed by Venture. The defaults are `http://127.0.0.1:3000` for Intel and `http://127.0.0.1:8080` for the API. Venture initializes the workspace, creates private credentials, and starts the Compose project. Use the same workspace for client commands and shutdown.

```sh
./target/debug/venture --workspace ./.fleetworks client jobs tui
./target/debug/venture --workspace ./.fleetworks logs --follow
# Press Ctrl+C to leave the log view before stopping the stack.
./target/debug/venture --workspace ./.fleetworks stop
```

Use `venture web` to start the stack with Jobs and Logs open in the terminal. Pass `--terminal logs` to follow service output instead. Closing the terminal view leaves the services running.

### Collect articles with Atlas

Start a new or stopped stack with `up --atlas` and submit a cycle through the Atlas client. Each new cycle needs a unique request ID.

```sh
./target/debug/venture --workspace ./.fleetworks up --atlas
./target/debug/venture --workspace ./.fleetworks client atlas start --request-id review-1
```

Set up the provider credentials and a Rust-BERT worker and model for the container architecture before collecting live articles. The [Atlas deployment guide](deploy/atlas-dispatch.md) lists the files and settings.

### Choose storage and import settings

Venture defaults to disk-backed SurrealDB in Compose. It can also manage a host SurrealDB process in disk or memory mode, or connect to an authenticated external disk server. Use `venture import --from` to copy existing provider keys and model defaults into a fresh workspace. See the [workspace guide](docs/wiki/code/fleetworks.md) for commands and storage settings.

## Applications and services

| Area | What you can do |
| --- | --- |
| Intel web | Browse the Bulletin and open Focus Briefs, Related articles, actor dossiers, and topology. |
| Atlas | Submit a cycle, check its job, and page through collected articles. |
| Brain | Read actor, brief, and topology details through the authenticated API. |
| Jobs and Logs | Review stages and events. Retry or cancel jobs through the terminal client. |
| Tools, Models, and Profile | Run a catalog tool, check its result, manage role defaults, and view workspace information and health. |

[![Fleetworks service boundaries](docs/diagrams/fleetworks.svg)](docs/diagrams/fleetworks.html)

Venture and the terminal clients use Rust, Tokio, and Clap. Axum serves the versioned HTTP API, and Next.js runs the React Native Web desktop through a same-origin relay. SurrealDB owns job, delivery, and application state. The Atlas overlay uses RabbitMQ for delivery and runs Compromise NLP and Rust-BERT behind private endpoints. See the [architecture](docs/wiki/code/architecture.md) and [rollout status](docs/wiki/code/fleetworks.md#rollout-status) for service ownership and open work.

## Development

Use the [contributor workflow](docs/wiki/code/contributing.md) to select checks for the code you changed. Preview the hosted checks for the current commit with the scope planner.

```sh
python3 scripts/fleetworks_verify.py plan --base "$(git rev-parse HEAD^)" --head "$(git rev-parse HEAD)"
```

Web contributors can find setup and verification in the [Intel web guide](docs/wiki/code/intel-web.md) and the [React test guide](docs/wiki/code/react-ui-testing.md). The [CI guide](docs/wiki/code/ci.md) covers hosted platform and package verification. Agents start at [AGENTS.md](AGENTS.md).

For more, browse the [concepts](docs/wiki/concepts/index.md), [code map](docs/wiki/code/index.md), or [diagram catalog](docs/diagrams.md).
