//! Demo content ids shared by host tests / client fallbacks.
//!
//! Production layout SSoT is Elixir `World.DemoScene`
//! (`world-server/lib/world/demo_scene.ex`). Keep ids in sync manually (x0030).

/// Coin / primary mesh entity id in DemoScene.
pub const DEMO_MESH_ENTITY_ID: u32 = 1;

/// First grabbable box entity id; boxes are `DEMO_BOX_ENTITY_BASE + 0..8` → 10..=18.
pub const DEMO_BOX_ENTITY_BASE: u32 = 10;

/// Coin blake3 hex used by DemoScene (assets fixture).
pub const DEMO_COIN_BLAKE3: &str =
    "fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a";
