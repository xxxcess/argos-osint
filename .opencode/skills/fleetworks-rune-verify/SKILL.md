---
name: fleetworks-rune-verify
description: Design, implement, and review Fleetworks Desktop on Linux x86-64 and macOS using Rune, Glass Canvas, accessible interactions, and inspected production-view fixtures. Use for native screens, components, motion, and interaction states. Intel Web uses its own React workflow.
---

# Fleetworks native design and Rune verification

Read root/scoped AGENTS, the existing [testing guidance](../argos-testing/SKILL.md) and [native verification guide](../../../docs/wiki/code/native-verification.md). Reuse authorized verification scope and one integration/build owner. Read the pinned Rune/kit README Agent tooling and Accessibility sections, hello example and tooling args when changing the harness; resolve the exact revision from Cargo.toml/Cargo.lock.

## Design the interaction

Read the [native design guide](../../../docs/wiki/code/native-design-system.md#designing-a-native-interaction) before selecting controls. Work from the operator's task, the relevant data and authority, and the action that advances it. Use the existing surface and disclosure pattern. Keep connection status, the selected item, and the next relevant action easy to find at minimum width.

- **Composition.** Use the approved [Glass Canvas theme](../../../docs/design/matte-canvas/theme.md) and the shared tokens, materials and controls in `theme.rs`, `material.rs`, and `ui_components.rs`. Build shell chrome, primary cards, Settings containers, controls, and floating overlays as layered smoked/frosted dark glass (`GlassSurface` + crisp static liquid-metal specular rim highlights, with deterministic opaque fallback under `ViewPreferences::matte()`), paired with warm molten-gold/cyan highlights and fractured crescent-moon rock textures on committing actions and brand accents over the `#060709` deep-space backdrop. Follow the calm desktop layout rhythm (slim smoked-glass left rail, centered glass command trigger, compact top-right telemetry/bell strip, hairline-grouped glass settings, floating frosted-glass Inbox popover stack). Use aligned lists for scanning, tables for comparison, and cards for groups that share a decision. Reserve status color for meaning and pair it with a label or icon.
- **Input and motion.** Give press, selection, and focus distinct feedback. Keep keyboard order, fixed hit bounds, Escape, and focus return when inserting a disclosure or overlay. Use existing Rune transitions only when they explain a change. The [motion contract](../../../docs/design/matte-canvas/motion.md) owns Quiet Orbit and its reduced-motion behavior.
- **States.** Design unresolved, settled, refreshing, unavailable, failed, and success states together. Keep resolved text, row identity, selection, and scroll position while an in-scope read refreshes. Localize progress or retry to the affected region and preserve explicit mutation confirmation. Follow the [interaction-state contract](../../../docs/design/matte-canvas/interaction-state.md).

## Verify the production view

Production and fixtures share `app::view`, `State`, theme and handlers. Use [native_fixture](../../../apps/fleetworks-desktop/examples/native_fixture.rs) synthetic effects for credentials, folders, network, dialogs, browser and clipboard; never point a scene at real user data. Assert observable state/input behavior with Rune Runtime before images. Add only affected finite regressions: stale completions, permissions, bounded pagination/follow, scope changes, unavailable/error states, keyboard/focus and protected filesystem behavior.

`Runtime::find` reports a drawn centre without checking clipping. For offscreen controls, use `Tx::scroll_into_view`, advance a frame and assert the centre is inside the actual scroll viewport before pointer input. `Runtime::window_focus` queues the element focus event; advance a frame before asserting its handler effects. Keep assertions on the real production controls.

For agent builds, inspect and reuse the matching `target/agents/debug` graph through `python3 scripts/agent_cargo.py … --locked`. Run scoped app/contracts/workspace tests as affected. Native view-only changes do not compile Linux engines, install Node/Docker or run the embedded compatibility matrix. Select backend/embedded checks when those contracts change.

Generate selected real-view evidence with the existing build-once helper:

```sh
python3 scripts/fleetworks_verify.py native-ui --agent \
  --scene brain-search-ready --size 1280x800 --scale 2 \
  --output .agent-scratch/native-ui/selected-review
```

Compare affected visual scenes with the approved `docs/design/matte-canvas/` Glass Canvas theme, motion and interaction/state targets. Check symbol-only crescent + molten-core branding (no orbit triangles), layered smoked/frosted glass surfaces across chrome/cards/overlays with crisp static liquid-metal rims (plus opaque `matte()` fallback), crescent-rock texture on Submit/Confirm/Create/Delete/Connect and brand accents, local loading sheen and stable continuous-scroll behavior. Generated concept mockups are targets, not Runtime evidence.

Choose scenes from the actual executable inventory; the helper rejects zero, unknown, missing and stale output. Use affected compact/wide, scale, loading/error, matte/material and accessibility states. Add an existing bounded input script and expected final tree key when interaction matters. Keep outputs ignored and preserve original failure diagnostics. Never use terminal cells or an image approximation as Rune evidence.

Open generated PNGs and inspect text, focus, clipping, scroll regions, contrast and state. Record generated versus inspected separately; repair demonstrated failures and rerun affected cases. Headless output does not prove Launch Services, real dialogs/clipboard, system preferences, minimum OS or matching-hardware GUI behavior. Report exact commands, executed cases, selected scene/profile/script, inspection findings and pending external checks. Do not claim signing, publication, live inference or backend integration from a native fixture.

For PR review, use the exact-head Linux x86-64 candidate and early PNG/tree evidence from [first-pass CI](../../../docs/wiki/code/ci.md#nightly-gsd-order). Record inspected states and live findings as evidence becomes available. When first-pass CI succeeds, dispatch applicable macOS and React Native Web checks for that head. Platform-specific live review can run alongside CI. Use [dispatchable visual review](../../../docs/wiki/code/native-verification.md#dispatchable-ui-visual-review) for additional affected Mac or web captures after the first pass. Keep synthetic captures, physical app behavior and transport verification distinct.
