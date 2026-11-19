// The rock tile for Submit, Confirm, Create, Delete and Connect.
//
// It is the approved crescent's coarse rock, derived once by
// assets/brand/source/derive_rock_tile.py (recipe: docs/design/matte-canvas/theme.md,
// "Deterministic asset and tint recipe") into assets/brand/rock/fleetworks-rock-tile.png and
// embedded here as a data URI, as FleetworksMark embeds its vector, so no asset URL or base path
// is involved. The overlay is 512 x 512 and shown at a fixed 256 x 256 logical pixels, repeated
// and never stretched. Nothing is generated at runtime. Native draws the same pixels from the
// committed .rgba beside it.

import { ROCK_TILE_DATA_URI } from "./rockTileData";

/** Logical size of one tile; the 512 px source is shown at half scale. */
export const ROCK_TILE = 256;

/** The tile as a CSS `url(...)` value. */
export const rockTileImage = `url("${ROCK_TILE_DATA_URI}")`;
