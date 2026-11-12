# Fleetworks native client

This macOS Rune/rune-kit client connects to an independently running Fleetworks gateway. It does not start the backend or Intel Web. Backend pages are Atlas, Brain, Tools, Models and Profile; Workstation pages are Workstation, Workspaces and Jobs & Logs.

The shared native views provide offline setup, authenticated capability checks, backend readiness, actor discovery and dossier, bounded article/tool/model/profile reads and registered workstation activity. Protected workspace startup, list/select/create/owned attach, Finder reveal and local operation journals run on blocking workers. Folder switches preserve global drafts and Brain state while invalidating future origin and pending confirmations.

Native IP comes from an explicit current LAN/VPN interface choice. A remote gateway route provides a suggestion; loopback never substitutes an external IP lookup. Registration is authenticated and validated against the selected deployment, folder marker, canonical receipt and context revisions. Unknown registration HTTP outcomes retain an exact durable retry intent and block folder removal until reconciled.

An explicit IP change previews its effect on every workspace and browser session on that backend. The exact request is journaled before HTTP; authenticated completion preserves stable workstation/workspace IDs and receipt history. Other children need a fresh registration before further work or removal. A lost response is retried from the saved request rather than replaced with a new operation.

After dependency/toolchain compatibility has been verified, build the scoped app with Cargo, or use the repository candidate helper to package a `.app`. Cargo alone produces an executable. Missing connection configuration opens setup; no model/catalog download is needed to display the shell. The native Metal implementation's source-based minimum is macOS 13; actual minimum-OS and both-architecture launch evidence remain separate.

```sh
cargo run --locked -p fleetworks-desktop --bin fleetworks-desktop
```

An explicit `--api-origin` and `--api-token-file` configure the connection editor; it connects only after the user chooses Connect. Credential files must be private, bounded regular files. Native `--workspace` accepts an existing owned child basename or its exact immediate-child path. The protected root, external directories, symbolic links and unowned legacy roots require repair; they are never adopted automatically.

Dataset transfer uses a chosen local regular CSV, an identity-checked file descriptor, bounded hashing on a blocking worker and the shared SDK byte stream. The backend verifies bytes/schema/hash and returns an opaque dataset reference. Maintenance previews pin registered context, configuration revision and the exact dataset; the native client treats a durable job receipt as acceptance, never completion. Seed, Natural training and actor reindex stay disabled unless the gateway advertises a real permitted consumer. Training does not activate a candidate.

Atlas collection and manual Tools submission require an advertised durable worker, interactive permission and a confirmed registered origin/configuration revision. An uncertain response keeps the exact request identity and input snapshot for reconciliation; edited drafts never replace a retry body. Returned IDs are opaque and are verified through registered Jobs. Tools results additionally require private-details eligibility; native views retain only bounded structured observations and safe status metadata, never raw provider content. A later configuration save does not hide an already accepted job or its authorized result.

## Remaining functional gates

- Atlas/Tools submission and receipt/result views require the current protocol checkpoint, scoped gateway journey and fixture verification. They stay disabled when the gateway has not enabled its genuine respective worker; catalog reads alone do not enable execution. The explicit read-only request lookup resolves retained uncertain handles under current proof and the original selected workspace, then verifies the returned Jobs identity without fabricating a lost POST receipt or submitting replacement work.
- Models/Profile forms send confirmed revision-pinned account, tool-credential and canonical-role changes. Explicit schema-4 credential-bearing export/import uses bounded SDK bytes and private create-new local files. Native forms/file guards passed the current local fixture/unit checkpoint; real gateway and installed-app journeys remain separate. Historical configuration is never imported automatically.
- Jobs uses only registered workstation activity and an optional narrowing workspace filter. Queued cancellation and failed Atlas retry require the server's specific availability flags, distinct permissions, and explicit confirmation; uncertain outcomes retain the same request ID. There is no global jobs/logs fallback. Optional maintenance results show only safe returned metadata; training never implies activation.
- Removal/delete confirmation and recovery journal preserve files while any known backend retirement is missing. The connected matching backend receives the retirement request; `retirement_pending`/`deletion_ready: false` keeps cleanup pending. Only an identity-matching response confirming admission fencing, session revocation and drained work records a retirement receipt. Each known backend must independently confirm.
- The guarded macOS Trash adapter persists a file-reference URL and verified per-volume user Trash identity before its effect. It checks the exact folder object/marker and original parent, then verifies that reference inside OS Trash and the old entry absent. Restart uncertainty remains pending; a replacement entry is never removed. Its isolated fake-adapter tests passed. No real user-folder Trash operation has been performed.
- Native Natural activation inspects an authorized completed training job's exact candidate hashes and expected prior model revision. Synthetic baselines require separate explicit consent. Confirmation pins the current registered origin and observed configuration revision, including explicit revision zero; uncertain delivery retains the exact request ID/body. Opaque acceptance IDs are verified through private authorized Jobs before reading the operation-owned result. `EffectUncertain` never becomes success from a matching current pointer, and activation does not imply inference readiness. Native tests passed; new activation captures and positive public API/full worker journeys remain pending. App packaging, installed GUI launch, signing/notarization and physical-Mac accessibility remain separate gates.

Development verification uses the same views and handlers with injected synthetic effects. Scene inventory includes offline states, registered Workstation, managed Workspaces, article/tool/model/profile data, scoped Jobs/private-shared status and maintenance preview:

```sh
cargo run --locked -p fleetworks-desktop --example native_fixture -- --list-scenes
cargo run --locked -p fleetworks-desktop --example native_fixture -- \
  --scene brain-search-ready --size 1280x800 --scale 2 --at 0 \
  --snap .agent-scratch/brain-search.png --tree .agent-scratch/brain-search.tree.json
```

Agents use `python3 scripts/agent_cargo.py … --locked`; coordinate verification scope and the shared build cache. Inspect the actual PNG and tree. Fixture generation does not establish a real gateway operation, installed app launch, signing/notarization or physical-Mac accessibility pass.

The lifecycle checkpoint passed 71 combined tests: 24 contracts, 26 native, 11 workspace unit and 10 workspace integration tests. Job cancel preview and retry receipt PNGs were inspected. This does not establish a real user-folder Trash journey.

The appearance/readiness increment adds five tests and 15 scenes (57 total). Production samples macOS Reduce Transparency on the UI thread, invalidates the per-window cache on focus and coalesces redraw reads. Rune's actual Reduce Motion preference is preserved. Fixtures use explicit preferences and never read or change the host settings. A static reflection is limited to the existing compact Connection toolbar control; reduced transparency uses its opaque matte fallback. Passive content stays matte. Readiness labels stale observations separately from their last reported state, including unknown inference and incomplete seed/training states. The combined 78-case checkpoint passed (26 contracts, 31 native, 21 workspace). Native/matte toolbar PNGs and the visible stale observation were inspected; the initial inference-unknown card was confirmed in its tree but was below the captured viewport. Actual OS preference journeys remain pending.

The exact-candidate activation checkpoint passed 84 combined tests: 26 contracts, 37 native and 21 workspace. Its six new scenes bring the inventory to 63; new activation capture evidence remains pending. These fixtures use production handlers with injected synthetic boundaries; they do not establish a complete gateway/worker or real inference journey.

The native library and renderer fixture are macOS-only. Other hosts expose a clear unsupported-host executable error so Linux workspace service checks can retain their existing graph. A Linux stub check is never native renderer or launch evidence; both real Intel and Apple Silicon macOS lanes remain required.
