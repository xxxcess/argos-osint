# Fleetworks native client

Read the root instructions, [workstation redesign handoff](../../.planning/2026-11-12-native-workstation-redesign/handoff.md) and [native design/control allowlist](../../docs/wiki/code/native-design-system.md) first. Use the repository Rust style skill. This app is a macOS Rune/rune-kit API client; the gateway and Linux workers own domain operations. Never restore retired launcher/terminal entry points or import terminal UI, storage/search/inference engines, Docker APIs, a webview, embedded HTTP server or process runner.

Production and verification share `app::view`, `State`, theme and handlers. The `native_fixture` example supplies only synthetic state/effects and must not read real homes, credentials or gateways, create/reset real folders, or launch a browser. Use Rune Runtime state/input assertions before PNG/tree inspection; generated or unviewed images are not visual approval. A missing renderer/adapter is blocked evidence, not success.

Load [Rune verification](../../.opencode/skills/fleetworks-rune-verify/SKILL.md) and use the [selected native verifier](../../docs/wiki/code/native-verification.md). Its `native-ui --agent` command builds once through the shared debug cache and validates the actual executable's scene inventory. Record inspection separately; do not reconstruct native UI from terminal cells.

Use the integration owner's approved scope and shared `scripts/agent_cargo.py … --locked` cache. Workers do not start competing builds. Native edits do not require host backend engines or terminal screenshots. Web owns topology and its accessible list; Brain is search/results/authoritative dossier only.

Keep backend resource scope separate from registered workstation origin. Jobs & Logs must remain server-enforced workstation-only activity, with no all-backend fallback. Normal startup opens protected installation settings without creating a default child. Existing folders/journals remain historical; do not expose retired folder/Trash workflows. Setup uses only Gateway URL and connectivity progression with exact-origin provisioned credential references. Offline setup remains usable.

Follow the [approved pairing/Keychain target](../../docs/wiki/code/native-connections.md#pairing-and-keychain-agreed-target) for new setup work. It is not implemented; current file provisioning is transitional.
