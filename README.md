# Argos OSINT

Argos is a terminal investigation workspace. It gathers public sources, binds evidence, and writes cited answers.

Home launches four applications — **Intel**, **Atlas**, **Brain**, **Recon** — and five system surfaces: **Jobs**, **Logs**, **Tools**, **Models**, **Profile**.

Only Recon has a chat. Everything else is fields, lists, and buttons. Observations are retrieved data with a timestamp. They are not proof of identity or ownership.

[Docs](docs/README.md) · [Usage](docs/wiki/concepts/usage.md) · [Figures](docs/diagrams.md) · [Releases](docs/wiki/code/versioning.md) · [Changelog](CHANGELOG.md)

[![Argos workspace](docs/diagrams/workspace.svg)](docs/diagrams/workspace.html)

## Applications

| App | What it does | Read more |
| --- | --- | --- |
| Intel | Browse Atlas headlines and prepare a cited Focus Brief with ingest-related articles | [Usage](docs/wiki/concepts/usage.md#intel) · [Brief](docs/diagrams/intel-brief-chain.html) |
| Atlas | Classifier-only news collection, country heat, local metadata and Related leads | [Usage](docs/wiki/concepts/usage.md#atlas) · [Cycle](docs/diagrams/atlas-pipeline.html) |
| Brain | Claims, unique Actor names and navigable associations with read-only graphs | [Usage](docs/wiki/concepts/usage.md#brain) · [Browse](docs/diagrams/brain-browse.html) · [Recall](docs/diagrams/brain-recall.html) |
| Recon | Evidence-driven investigations with a planner, binder, and synthesis | [Usage](docs/wiki/concepts/usage.md#recon) · [Architecture](docs/wiki/code/architecture.md) · [Turn](docs/diagrams/recon-turn.html) |
| Tools | Manual OSINT catalog (internal id `Osint`) | [Providers](docs/wiki/concepts/providers.md) · [Catalog](docs/diagrams/osint-providers.html) |
| Models | Role defaults and OpenRouter / Google / Nvidia accounts (`Providers`) | [Providers](docs/wiki/concepts/providers.md) · [Roles](docs/diagrams/model-roles.html) |
| Jobs / Logs | Background work and durable events | [Usage](docs/wiki/concepts/usage.md#other-apps) · [Jobs](docs/diagrams/jobs-lifecycle.html) |
| Profile | Host hardware and storage paths (`System`) | [Concepts](docs/wiki/concepts/glossary.md) |

## Quick start

Rust **1.95+** (this repo pins **1.95**). First build needs a C toolchain (`xcode-select --install` on macOS). Protoc is vendored.

```sh
cargo run -p argos-osint-bin
```

A default build stores everything in embedded SurrealDB: RocksDB on disk under
`~/.argos/storage/surreal` (`ARGOS_HOME` moves it), in-process, with no network.
To use a per-home SurrealDB 3.3.4 server on 127.0.0.1 instead, build with
`--no-default-features --features server,nlp` and run `argos storage init --mode server`
(or set `ARGOS_STORAGE_MODE=server`).

From Home: `↑↓` and Enter open an app. Keys `1`–`9` jump to Intel through Profile. Esc returns Home. `?` opens shortcuts.

```sh
cargo build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # offline; ARGOS_EMBED unset
ARGOS_EMBED=1 cargo test -p argos-osint-core -- --ignored minilm
scripts/local_matrix.sh --storage embedded               # what CI runs, default build
scripts/local_matrix.sh --storage server --surreal-bin /path/to/surreal
```

Navigation, CLI, and state directory: [docs/wiki/concepts/usage.md](docs/wiki/concepts/usage.md). Agent guidance: [AGENTS.md](AGENTS.md).

## Figures

Open in a browser for the HTML page, or view the SVG inline.

[![One Recon turn](docs/diagrams/recon-turn.svg)](docs/diagrams/recon-turn.html)

[![Persistence stack](docs/diagrams/persistence.svg)](docs/diagrams/persistence.html)

[![Intel report job](docs/diagrams/intel-report.svg)](docs/diagrams/intel-report.html)

The rest of the set: [docs/diagrams.md](docs/diagrams.md).

## Documentation

Start at the [repo wiki](docs/index.md): choose [concepts](docs/wiki/concepts/index.md) to understand Argos or [code](docs/wiki/code/index.md) to explore its implementation. Agent navigation starts at [AGENTS.md](AGENTS.md).

## Limits

- Adapters issue bounded public HTTP. A catalog entry or fixture test does not prove live availability.
- Shodan InternetDB is noncommercial.
- Nominatim needs an identifying User-Agent, attribution, caching, and at most one request per second.
- Model calls use the account and terms you configure.
