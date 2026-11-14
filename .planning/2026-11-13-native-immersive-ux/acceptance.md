# Native Immersive UX Overhaul — Verification & Acceptance

[Native design](../../docs/wiki/code/native-design-system.md) · [Native verification](../../docs/wiki/code/native-verification.md) · [Design blueprint](design.md) · [Decisions](decisions.md)

This record separates implemented source/documentation from verified Rust tests, inspected Runtime PNG/tree evidence, binary builds, and live GUI/backend readiness. All desktop/workstation/contract, doc-policy, and visual PNG gates passed on the frozen source after `#24`/`#29`/`#34` (the full `argos-osint-api --lib` run has 2 pre-existing unrelated SurrealDB test failures noted below).

| Area | Command / Scope | Status |
| --- | --- | --- |
| Documentation & diagram links | `python3 scripts/fleetworks_doc_links.py` | PASS (`Checked 1936 relative documentation references, 0 broken`) |
| Repository policy tests | `node --test .opencode/tests/*.test.mjs` | PASS (`21/21` passed) |
| Formatting | `cargo fmt -p fleetworks-desktop -p argos-osint-api -- --check` | PASS (exit `0` on `#24`/`#29`/`#34` frozen source) |
| Scoped Clippy (desktop) | `python3 scripts/agent_cargo.py clippy --locked -p fleetworks-desktop --all-targets -- -D warnings` | PASS (`0` warnings on `#29`/`#34` frozen source) |
| Scoped Clippy (API probe fix `#24`) | `python3 scripts/agent_cargo.py clippy --locked -p argos-osint-api -p argos-api-host --no-default-features --features server,nlp,classifier -- -D warnings` | PASS (`0` warnings on `#24` frozen source) |
| Scoped Rust tests (desktop/workstation/contracts) | `python3 scripts/agent_cargo.py test --locked -p fleetworks-desktop -p fleetworks-workstation -p fleetworks-contracts -- --test-threads=1` | PASS (`179 passed, 0 failed, 0 ignored`: `fleetworks-contracts` 48, `fleetworks-desktop` lib 95 / bin 0, `fleetworks-workstation` lib 26 / `native_workspaces` 10) |
| Scoped Rust tests (`argos-osint-api --lib`, `#24`) | `python3 scripts/agent_cargo.py test --locked -p argos-osint-api --lib` & `-- run_shielded` | PARTIAL (`3/3` `run_shielded` passed + `workstation::tests::*` and `live::tests::*` passed; full `--lib` ran `68 passed, 2 failed` due to 2 pre-existing unrelated SurrealDB failures in `datasets::upload_tests` (`upload_tests.rs:307`) and `maintenance::tests` (`maintenance/tests.rs:343`)) |
| Native Runtime captures & visual inspection | `python3 scripts/fleetworks_verify.py native-ui --agent` | PASS (`@ux-verifier` inspected all 13 fresh PNGs under `.agent-scratch/native-ui/selected-gol4tlxd/` — `workstation-offline`, `redesign-setup`, `redesign-status` + `CPU Telemetry` modal, `redesign-seed`, `redesign-brain`, `redesign-jobs`, `pairing-waiting`, `pairing-connected` + advanced toggle, `appearance-native` + hover, `appearance-matte` — alongside prior 19 baseline PNGs in `selected-8_27o2uj/` + `selected-9820_hur/`) |
| Native binary & backend runtime readiness | `python3 scripts/agent_cargo.py build --locked -p fleetworks-desktop --bin fleetworks-desktop` + read-only host stack check | PASS (`target/agents/debug/fleetworks-desktop`, `116M`; `df -h .` = `49Gi` free; `fleetworks-native-review-native-local` 11/11 containers `Up` with `http://127.0.0.1:18091/api/v1/health` responding `{"error":"API authentication required"}`; note: running container images predate `#24` local source fix until rebuilt) |

## Implemented boundaries & verification notes

- **Locked Rune pipeline (Phase 2B)**: Retained `--locked` `rune` + `rune-kit` + `rune-motion` dependencies (`Cargo.lock` unchanged; no `rune-fx` or `rune-light` addition).
- **Cache hygiene**: Safely reclaimed 54.14 GiB (49,495 files / 456 dirs of provably obsolete artifacts under `target/agents/debug/{deps,incremental,.fingerprint}`), leaving 55 GiB available (`df -h .`) without touching `target/debug`, global caches, or `.agent-scratch/`.
- **Palette & icons**: Brand-anchored obsidian/ivory/gold palette and `StatusTone` semantic/entity tokens in `apps/fleetworks-desktop/src/theme.rs`; 19 custom 16×16 `rune::Icon` SVG glyphs in `apps/fleetworks-desktop/src/icons.rs`.
- **Liquid metal, dim pointer lighting & matte parity**: Reduced static metallic sheen across `MetalFinish` (`GoldCta`, `Gunmetal`, `CrimsonDanger`, `ObsidianPill`), `ActionButton`, `luminous_card`, and `interactive_tile` (`material.rs`, `ui_components.rs`) with `Live<[f32; 2]>` dim pointer lighting (`liquid-pointer-light`, `liquid-specular`, `pointer-glow`, and opposing `cast-shadow` updated via `tx.live_only()` in `app.rs`), preserving `matte()` opaque flat fallback and identical hit-bounds (`appearance_tests.rs`).
- **Transitions, offline gating & sidebar**: `Page::Workstation` default home on startup/disconnect; connection-gated sidebar shows only `nav-workstation` + `nav-setup` when disconnected (removed `sidebar-context-footer`), `views::page`/`toolbar`/`screen_key` clamp to `Workstation Status`, `host-resources-card` and `readiness_cards::checks` stay hidden offline, and `scope_bar` shows `"Local Client"` (`local-client-scope`). Connected views retain `Transition::offset + opacity` (`reduced_motion` disables animations), `async_status_strip`, `metric_ring_tile`, `stage_stepper`, `sidebar_row_status`, and `sidebar_row_badge`.
- **Progressive disclosure & modal inspectors**: `SeedStage`, `ModelsTab`, `DossierFacet`, `WorkstationInspector`, `Checkbox` preflight checklist (`checklist_row`), `Switch` inline toggles (`inline_toggle_row`), and `Dialog` (`modal_prompt`) inspectors for host telemetry, news cycles, job summaries, and OSINT tool API keys.
- **Visual fixes verified on rerun**: Moved dialog description prose into wrapped modal body (`ui_components.rs`, `backend_views.rs`), added `.min_w(0.0).grow(1.0)` + wrapped subtitle in `section_header` and raised `actor-results` `.min_w(300.0)` with `.gap(16.0)` (`views.rs`), and widened `safe_log_table` `severity` column to `width(92.0).min(88.0)` (`backend_views.rs`).

## Limits

- Synthetic fixture effects and `Runtime` captures do not establish installed-app macOS window behavior, real macOS Keychain authorization dialogs, code signing/notarization, live provider API calls, or hosted dual-Mac CI (`macos-15-intel` / `macos-15`).
- `docs/diagrams/native-workstation-layout.{svg,html}` is a layout/control schematic, not a Runtime screenshot.
- Launching `target/agents/debug/fleetworks-desktop` interactively and approving pairing against `http://127.0.0.1:18091` are reserved for the user.
