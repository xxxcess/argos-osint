# Fleetworks native client

The Rune/rune-kit macOS API client connects to an independently running gateway. It never starts backend engines, Docker or Intel Web. Its six sidebar destinations are DB Seed & Train, News Cycles, Brain Search, OSINT Tools, Model Assignments and Jobs & Logs. The gateway header opens Workstation Status, with Native Client, Gateway Backend and Host Resources cards. Setup Gateway opens URL → operator approval → Keychain → authenticated checks/registration.

[Design and control allowlist](../../docs/wiki/code/native-design-system.md) · [Connection contracts](../../docs/wiki/code/native-connections.md) · [Protected identity](../../docs/wiki/code/native-workspaces.md) · [Implementation/verification status](../../.planning/2026-11-12-native-pairing-implementation/acceptance.md)

[![Six destinations and selected content](../../docs/diagrams/native-workstation-layout.svg)](../../docs/diagrams/native-workstation-layout.html)

## Build and provision

Normal setup uses [pairing and macOS Keychain](../../docs/wiki/code/native-connections.md#pairing-and-keychain-agreed-target); no token file is needed. Verification remains separately tracked.

Use the complete-app helper; agent builds add `--agent` and reuse the shared debug graph. Opening the app is an explicit action.

Build flow: live Cargo progress/diagnostics and stage markers on stderr → package/validate → artifact paths and result metadata. `--open` opens only after validation and preserves an already running app. [Build loop](../../docs/diagrams/dev-loop-native.html)

```sh
python3 scripts/fleetworks_candidate.py build-native --agent --target host --profile debug
```

For an existing local review, the backend operator first runs:

```sh
python3 deploy/native_review.py up --name native-local
```

This command preserves existing credentials/data, adds missing Operator provisioning, validates local Docker/configuration, builds the configured images with live output, then restarts only that review after a successful build and waits for readiness. On macOS it then builds/opens the app using `build-native --target host --profile debug --open`; native build failure leaves the healthy backend running. Add `--backend-only` to skip the app step; non-macOS hosts skip it automatically. It prints the actual gateway URL and approval command. Use `prepare --name native-local` for an offline provisioning/command preview instead. For a new deployment, [initialize it first](../../deploy/README.md#initialize-a-fresh-review). [Backend review workflow](../../deploy/README.md#prepare-an-existing-review)

Open Setup Gateway, enter the helper's gateway URL and choose Connect. Share the displayed pairing request ID/code with the [gateway operator](../../docs/wiki/code/native-connections.md#operator-approval-and-revocation). The app authenticates approved access, saves it in macOS Keychain and registers the installation. Later connections use that exact origin/deployment; missing/denied access has no file fallback. Retry preserves an uncertain request; Pair again explicitly replaces it.

Protected settings create no default child and contain no polling/access secrets. The opaque workstation ID is the browser Connect ID, not a credential. Optional IP metadata never defines identity; WebRelay remains independent.

**Operator/headless option:** explicitly launch the executable with both file-mode flags:

```sh
/path/to/Fleetworks.app/Contents/MacOS/fleetworks-desktop \
  --api-origin https://gateway.example.test \
  --api-token-file /absolute/private/native-token
```

Add `--provision-gateway-credential` to save a protected exact-origin file reference and exit without GUI/network/service startup. This optional catalog operation does not configure normal GUI pairing. Files must remain private, bounded, single-link regular files; reprovision after replacement/rotation. Existing operator files remain untouched.

## Selected views and durable actions

Backend resources stay shared within caller permissions. New registration/context/activity contracts have no workspace narrowing; existing folders and immutable historical receipts are preserved. There is no active folder/Trash workflow, saved-profile CRUD, account/base-URL administration or import/export screen.

[![Readiness and confirmed maintenance flow](../../docs/diagrams/workstation-readiness.svg)](../../docs/diagrams/workstation-readiness.html)

**DB Seed & Train:** each CSV picker first checks required columns locally, before any network request or backend worker. A valid file proceeds through a gateway-issued upload URL → streamed bytes → backend schema/hash/size validation and immutable staging → explicit job confirmation. Progress/errors stay visible; an invalid header stops locally. [Required columns, upload rules and sample files](../../deploy/README.md#configured-native-maintenance-inputs)

Actor CSVs merge by stable actor ID: add actors, update selected fields on matching actors and keep actors absent from the new CSV. Existing/manual aliases remain; new aliases are upserted without deletions, with full source metadata retained. Each job uses its exact selected dataset/hash/revision.

**Seed Actors** must complete with actors in the database before **Build Search Index** enables; queued/active reseeding keeps it disabled. Confirm the index job, then wait for reader readiness. **Train Classifier — Natural** uses its own CSV picker and produces a candidate without activation; the bundled v2 dataset is synthetic. Confirmation pins the actual configuration revision (fresh **0** is valid). Focused auto-refresh and **Refresh checks** only observe readiness; no seed/training job starts before confirmation. Authentication-bound, single-use upload tickets survive gateway restarts in the shared database until five-minute expiry; HTTP 429 leaves the URL unused. Accepted jobs remain durable. [Capacity and shared-staging requirements](../../deploy/README.md#configured-native-maintenance-inputs)

After updating maintenance code, close an already running Fleetworks app, then run `python3 deploy/native_review.py up --name native-local` to coordinate schema 11 bootstrap/services and rebuild backend and native app. The helper's `--open` preserves an existing running instance, so leaving it open would retain the older app.

News Cycles uses authoritative history/counts and workstation history tombstones; shared evidence and audit remain. Brain is one search field, bounded results and a dossier. Tools are service/provider cards with a key modal; manual tool runs/results are outside the native surface. Models provides inline keys and supported role-routing controls while preserving hidden fields/secrets and revision conflicts. Jobs & Logs shows server-enforced workstation activity and safe selected-job summaries without filters, action toolbars or raw provider output.

Durable acceptance is queued, not completed. Uncertain requests retain the exact request/body/context and original registration for read-only reconciliation. Changing a gateway fences old responses, caches and confirmations. Missing optional services disable only affected actions. Unknown values remain unavailable; app navigation never submits backend work.

## Verification

Use the engine-free app/contracts/workstation loop for native changes; storage compatibility tests belong to the heavier backend loop. Choose commands and preflight from [contributing](../../docs/wiki/code/contributing.md#development-loops-local-cli-and-docker); the [artifact/path map](../../docs/wiki/code/build-artifacts.md) distinguishes app output, QA harnesses and persistent data.

Production and fixtures share State, Rune views and handlers. Fixtures inject synthetic effects and do not access user homes, gateways, browser or clipboard. Use selected actual Runtime scenes, wide/compact sizes, PNG/tree/script inspection and separate installed-app evidence:

```sh
python3 scripts/fleetworks_verify.py native-ui --agent --list-scenes
python3 scripts/fleetworks_verify.py native-ui --agent \
  --scene brain-search-ready --size 1180x780 --size 640x480 \
  --scale 2 --output .agent-scratch/native-ui/brain-review
```

Discover scene names from the current executable; generation alone is not visual approval. [Native verification](../../docs/wiki/code/native-verification.md) records build/cache isolation and inspection requirements. Hosted evidence retains both Mac architectures; minimum OS, installed GUI/input/accessibility, signing/notarization and real backend integration remain separate. Read the acceptance tracker for actual pass/fail/skip results rather than treating these command examples as executed checks.

Selected PR checks build debug apps and Rune evidence on Intel/Apple Silicon, without ZIPs or publication. Trusted branch pushes package versioned release ZIPs, verify downloads by artifact ID/digest/checksum and publish after required checks: `nightly` → nightly prereleases, `dev`/`main` → stable. These automatically published apps remain unsigned; signing/notarization and installed-hardware acceptance are separate evidence. [Release flow](../../docs/diagrams/native-release.html) · [Artifact map](../../docs/wiki/code/build-artifacts.md#hosted-artifacts-and-publication)

Pairing tests use injected Keychain stores. Installed-app launch, real Keychain approval/reconnect and backend integration need separate evidence within the current task's authorized scope. Record actual results and remaining gaps in its acceptance tracker.
