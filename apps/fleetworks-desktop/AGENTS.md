# Fleetworks native client

Read root `AGENTS.md`, `argos-rust-style`, and [fleetworks-rune-verify](../../.opencode/skills/fleetworks-rune-verify/SKILL.md). The [native design guide](../../docs/wiki/code/native-design-system.md) is the control allowlist and links current Glass Canvas contracts.

- **Scope:** Rune client for Linux x86-64 and macOS. Brain owns search, results, and authoritative dossier; Jobs & Logs uses server-enforced workstation scope; Intel Web owns topology and its accessible list. Keep retired entry points, backend engines, Docker APIs, and embedded HTTP servers out of the app.
- **Backends:** Start the Embedded companion only on explicit user action. Distributed setup pairs installations to the gateway with platform secret-store credentials (explicit session-only option where supported). Preserve parity per [connection contracts](../../docs/wiki/code/native-connections.md#backend-types-and-parity).
- **Verify:** Exercise affected state/input in production `app::view` via synthetic Runtime fixtures, injected secret stores, and isolated homes; keep fixture builds engine-free. Inspect PNGs/trees and record them separately from installed-app or real secret-store evidence.
- **Review:** Use [Linux package and fixture artifacts](../../docs/wiki/code/ci.md#native-app-review-evidence) for pass one; once exact-head CI is green, dispatch macOS and Web checks in GitHub Actions. Generate GSD visuals in CI when possible; use the [selected verifier](../../docs/wiki/code/native-verification.md) and shared debug cache for authorized local runs.
