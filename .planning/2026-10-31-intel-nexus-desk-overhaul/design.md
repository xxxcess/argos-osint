# Intel Nexus Desk — implemented design

Updated 2026-11-01. This curated record replaces the initial overlay/orbit proposal. The maintained contracts are in [Intel web](../../docs/wiki/code/intel-web.md) and [React UI](../../docs/wiki/code/react-ui.md).

- Home, Bulletin, Brief and Topology are separate routes. Topology uses `seed`, containing the canonical Brain memory ID. Brief entity navigation is read-only; unavailable bindings wait for indexing.
- The browser calls same-origin Next REST and WebSocket endpoints. Only the server knows `ARGOS_RUST_API_ORIGIN`. Next relays frames without owning subscriptions or durable application state; Rust owns host sampling and pending-brief progress.
- REST handles snapshots, search, pagination and actions. A shared socket streams metrics and pending extraction/indexing progress. Completed briefs stop watching; reconnect resynchronizes the selected brief. Offscreen feeds do not poll.
- Shared React Native primitives retain the terminal desk palette. Browser density is 75%; native primitives keep their normal scale. Brief images preserve intrinsic aspect ratio. Bulletin cards have one activation target and separated category/date metadata.
- `ForceCanvas` keeps `react-force-graph-3d` with CSS2D glyph tiles and an accessible list twin. Actor, claim, inference and link kinds use distinct amber, cyan, violet and rose colors with matching legend icons. Topic nodes are excluded.
- A selection shows only itself and direct neighbors by default. Second degree is explicit; no disconnected ghost nodes participate. The graph fills the page with a compact Neighborhood HUD and no axes or pause/filter/zoom toolbar.
- Entry and new selections animate a fit of the first neighborhood. Subsequent user camera changes persist across data refreshes and renders. Reduced motion fits immediately. Label placement reserves space around nodes, captions, edge labels and the HUD.
- The selected tile glows. A pinned dependency patch guards duplicate drag cancellation without removing node interaction.

Reference screenshots remain design inputs, not proof of pixel equivalence. Performance and WebGL context-count targets require separate measurement; see [acceptance](acceptance.md).
