//! Small shared play-mode state resources (plan5), kept in one place so the
//! movement, floor-item, data-screen and hazard systems don't have to depend on
//! each other just to read a flag.

use bevy::prelude::Resource;

/// Whether the data screen (inventory/status overlay) is open. The world keeps
/// simulating while it's up (project.md / plan5「データ画面中もゲームは進む」);
/// only player movement and world pickup are suspended.
#[derive(Resource, Default)]
pub struct DataScreen {
    pub open: bool,
}

/// Which party member the data screen shows and which one picks items up. Clamped
/// to the party size by the systems that use it.
#[derive(Resource, Default)]
pub struct SelectedMember {
    pub index: usize,
}

/// Which data-screen tab is showing (plan7). `false` = items/equipment (the
/// plan5 view), `true` = the magic tab. The M key / 魔法 icon set this true.
#[derive(Resource, Default)]
pub struct DataView {
    pub magic: bool,
}
