//! Floor items (plan5): the 3D representation of items lying in the dungeon,
//! plus the pick-up / put-down operations.
//!
//! Each placed item is one entity carrying a [`FloorItem`] plus either a glTF
//! `SceneRoot` (when its def names a model) or a generic spinning gem tinted by
//! kind (so *something* is always visible). Pick-up/place go through the
//! selected party member's [`Inventory`].

use bevy::prelude::*;

use crate::character::Party;
use crate::dungeon::GridPos;
use crate::game_state::SelectedMember;
use crate::hud::MessageLog;
use crate::item::{ItemCatalog, ItemDef, ItemInstance, ItemPlacement, SlotRef};
use crate::player::Player;
use crate::render::BLOCK_SIZE;

/// Item placements to spawn at startup (from the loaded level). Inserted by
/// `main`; consumed by [`setup_floor_items`].
#[derive(Resource, Default)]
pub struct InitialItems(pub Vec<ItemPlacement>);

/// A pick-up request (fired by the `G` key / a `Get` script command).
#[derive(Event)]
pub struct PickupRequest;

/// A put-down request from the data screen: drop the item in `slot` of the
/// selected member at the party's feet.
#[derive(Event)]
pub struct PlaceRequest {
    pub slot: SlotRef,
}

/// An item lying on the floor.
#[derive(Component)]
pub struct FloorItem {
    pub instance: ItemInstance,
    pub pos: GridPos,
}

/// Marks the generic (no-model) gem so it can be spun for legibility.
#[derive(Component)]
pub struct SpinItem;

/// Model scale for item glbs (KayKit props read well near this size).
const MODEL_SCALE: f32 = 0.3;

fn tile_center(pos: GridPos) -> Vec3 {
    Vec3::new(
        pos.x as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
        pos.floor as f32 * BLOCK_SIZE,
        pos.y as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
    )
}

/// Spawn one floor-item entity (glTF model or generic gem) for `instance`.
/// Public so monster drops / thrown items (plan6) reuse the same visual.
pub fn spawn_loose_item(
    commands: &mut Commands,
    asset_server: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    def: &ItemDef,
    instance: ItemInstance,
    pos: GridPos,
) {
    let base = tile_center(pos);
    let mut entity = commands.spawn((
        FloorItem { instance, pos },
        Visibility::default(),
        crate::world::LevelScoped,
    ));
    if def.model.is_empty() {
        // Generic gem: a small tinted cube tilted to read as a diamond, hovering
        // and spinning so it's legible as a pickup.
        let (r, g, b) = def.kind.color();
        let mesh = meshes.add(Cuboid::new(0.16, 0.16, 0.16));
        let mat = materials.add(StandardMaterial {
            base_color: Color::srgb(r, g, b),
            emissive: LinearRgba::rgb(r * 0.3, g * 0.3, b * 0.3),
            ..default()
        });
        entity.insert((
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            Transform::from_translation(base + Vec3::new(0.0, 0.4, 0.0))
                .with_rotation(Quat::from_euler(EulerRot::XYZ, 0.6, 0.0, 0.6)),
            SpinItem,
        ));
    } else {
        entity.insert((
            SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(def.model.clone()))),
            Transform::from_translation(base).with_scale(Vec3::splat(MODEL_SCALE)),
        ));
    }
}

/// Spawn every placed item at startup.
pub fn setup_floor_items(
    mut commands: Commands,
    initial: Res<InitialItems>,
    catalog: Res<ItemCatalog>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for placement in &initial.0 {
        let Some(def) = catalog.get(&placement.id) else {
            continue;
        };
        let pos = GridPos::new(placement.x, placement.y, placement.floor);
        spawn_loose_item(
            &mut commands,
            &asset_server,
            &mut meshes,
            &mut materials,
            def,
            ItemInstance::new(placement.id.clone()),
            pos,
        );
    }
}

/// Slowly spin generic gems so they read as pickups.
pub fn spin_floor_items(time: Res<Time>, mut items: Query<&mut Transform, With<SpinItem>>) {
    for mut t in &mut items {
        t.rotate_y(time.delta_secs() * 1.5);
    }
}

/// Handle pick-up requests: take the item under the party's feet (or the tile
/// directly ahead) into the selected member's inventory.
#[allow(clippy::too_many_arguments)]
pub fn handle_pickup(
    mut requests: EventReader<PickupRequest>,
    mut commands: Commands,
    player: Res<Player>,
    selected: Res<SelectedMember>,
    mut party: ResMut<Party>,
    catalog: Res<ItemCatalog>,
    mut log: ResMut<MessageLog>,
    items: Query<(Entity, &FloorItem)>,
) {
    for _ in requests.read() {
        // Prefer the tile the party stands on, then the tile it faces.
        let (dx, dy) = player.facing.delta();
        let front = GridPos::new(player.pos.x + dx, player.pos.y + dy, player.pos.floor);
        let target = items
            .iter()
            .find(|(_, it)| it.pos == player.pos)
            .or_else(|| items.iter().find(|(_, it)| it.pos == front));
        let Some((entity, floor_item)) = target else {
            log.push("ここには何もない");
            continue;
        };
        let Some(member) = party.members.get_mut(selected.index) else {
            continue;
        };
        let name = catalog
            .get(&floor_item.instance.def_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| floor_item.instance.def_id.clone());
        match member.inventory.pickup(floor_item.instance.clone()) {
            Ok(_) => {
                commands.entity(entity).despawn_recursive();
                log.push(format!("{name}を拾った"));
            }
            Err(_) => log.push(format!("{name}を持ちきれない")),
        }
    }
}

/// Handle place requests from the data screen: remove the item from the selected
/// member's slot and drop it at the party's feet.
#[allow(clippy::too_many_arguments)]
pub fn handle_place(
    mut requests: EventReader<PlaceRequest>,
    mut commands: Commands,
    player: Res<Player>,
    selected: Res<SelectedMember>,
    mut party: ResMut<Party>,
    catalog: Res<ItemCatalog>,
    mut log: ResMut<MessageLog>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for req in requests.read() {
        let Some(member) = party.members.get_mut(selected.index) else {
            continue;
        };
        let Some(instance) = member.inventory.take(req.slot) else {
            continue;
        };
        let Some(def) = catalog.get(&instance.def_id) else {
            continue;
        };
        let name = def.name.clone();
        let def = def.clone();
        spawn_loose_item(
            &mut commands,
            &asset_server,
            &mut meshes,
            &mut materials,
            &def,
            instance,
            player.pos,
        );
        log.push(format!("{name}を置いた"));
    }
}
