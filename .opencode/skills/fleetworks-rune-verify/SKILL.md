---
name: fleetworks-rune-verify
description: Implement and verify Fleetworks Rune macOS views, state, input and accessibility with same-view Runtime fixtures and inspected PNG/tree evidence. Use for apps/fleetworks-desktop; retained terminal and React views use their own workflows.
---

# Fleetworks Rune verification

Read root/scoped AGENTS, the existing [testing guidance](../argos-testing/SKILL.md) and [native verification guide](../../../docs/wiki/code/native-verification.md). Reuse authorized verification scope and one integration/build owner. Read the pinned Rune/kit README Agent tooling and Accessibility sections, hello example and tooling args when changing the harness; resolve the exact revision from Cargo.toml/Cargo.lock.

Production and fixtures share `app::view`, `State`, theme and handlers. Use [native_fixture](../../../apps/fleetworks-desktop/examples/native_fixture.rs) synthetic effects for credentials, folders, network, dialogs, browser and clipboard; never point a scene at real user data. Assert observable state/input behavior with Rune Runtime before images. Add only affected finite regressions: stale completions, permissions, bounded pagination/follow, scope changes, unavailable/error states, keyboard/focus and protected filesystem behavior.

`Runtime::find` reports a drawn centre without checking clipping. For offscreen controls, use `Tx::scroll_into_view`, advance a frame and assert the centre is inside the actual scroll viewport before pointer input. `Runtime::window_focus` queues the element focus event; advance a frame before asserting its handler effects. Keep assertions on the real production controls.

For agent builds, inspect and reuse the matching `target/agents/debug` graph through `python3 scripts/agent_cargo.py … --locked`. Run scoped app/contracts/workspace tests as affected. Native view-only changes do not compile Linux engines, install Node/Docker or run the embedded compatibility matrix. Select backend/embedded checks when those contracts change.

Generate selected real-view evidence with the existing build-once helper:

```sh
python3 scripts/fleetworks_verify.py native-ui --agent \
  --scene brain-search-ready --size 1280x800 --scale 2 \
  --output .agent-scratch/native-ui/selected-review
```

Choose scenes from the actual executable inventory; the helper rejects zero, unknown, missing and stale output. Use affected compact/wide, scale, loading/error, matte/material and accessibility states. Add an existing bounded input script and expected final tree key when interaction matters. Keep outputs ignored and preserve original failure diagnostics. Never use terminal cells or an image approximation as Rune evidence.

Open generated PNGs and inspect text, focus, clipping, scroll regions, contrast and state. Record generated versus inspected separately; repair demonstrated failures and rerun affected cases. Headless output does not prove Launch Services, real dialogs/clipboard, system preferences, minimum OS or matching-hardware GUI behavior. Report exact commands, executed cases, selected scene/profile/script, inspection findings and pending external checks. Do not claim signing, publication, live inference or backend integration from a native fixture.
