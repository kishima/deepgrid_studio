//! Showcase props: a few CC0 KayKit models (provenance: CREDITS.md) placed in
//! the test level so enemies and items are visible in the dungeon.
//!
//! plan5/plan6 will replace this hardcoded list with data-driven item/monster
//! placement from the map file; this module exists so the look of the game can
//! be evaluated early. Positions are tuned to the sample project's level 0
//! (`assets/projects/sample/levels/level00.ron`).

use bevy::prelude::*;

use crate::render::BLOCK_SIZE;

/// Animation to play on a glTF scene once its `AnimationPlayer` shows up.
/// Attached to the `SceneRoot` entity; `attach_prop_animations` finds it by
/// walking up from the spawned player entity.
#[derive(Component)]
pub struct PropAnimation {
    graph: Handle<AnimationGraph>,
    node: AnimationNodeIndex,
}

/// KayKit models are authored at human scale (roughly 1.5–1.8 units tall);
/// our blocks are 1.0 — scale everything down to read well inside a cell.
/// Values tuned by eye against the `props` debug shot.
const SKELETON_SCALE: f32 = 0.45;

/// Animation indices in the KayKit skeleton glbs (identical rig/animation list
/// in every character of the pack — 95 clips).
const ANIM_IDLE: usize = 40;
const ANIM_IDLE_COMBAT: usize = 42;

/// Build a `(SceneRoot, PropAnimation)` for the glTF at `path`, set up to loop
/// animation `anim_index` once its `AnimationPlayer` appears (started by
/// [`attach_prop_animations`]). Shared by the showcase props here and plan4's
/// portrait rigs (portrait.rs).
pub fn animated_scene(
    asset_server: &AssetServer,
    graphs: &mut Assets<AnimationGraph>,
    path: &str,
    anim_index: usize,
) -> (SceneRoot, PropAnimation) {
    let scene = SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(path.to_string())));
    let (graph, node) = AnimationGraph::from_clip(
        asset_server.load(GltfAssetLabel::Animation(anim_index).from_asset(path.to_string())),
    );
    (
        scene,
        PropAnimation {
            graph: graphs.add(graph),
            node,
        },
    )
}

/// World translation for standing on the floor surface of tile `(x, y, floor)`.
fn on_floor(x: i32, y: i32, floor: usize) -> Vec3 {
    Vec3::new(
        x as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
        floor as f32 * BLOCK_SIZE,
        y as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
    )
}

/// Spawn the showcase models (two animated skeletons, a chest, a barrel).
pub fn setup_props(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let mut animated = |path: &'static str, anim_index: usize| -> (SceneRoot, PropAnimation) {
        animated_scene(&asset_server, &mut graphs, path, anim_index)
    };

    // Skeleton minion on floor 1, two tiles south of the start, facing the
    // player's spawn (north = -Z).
    let (scene, anim) = animated("models/enemies/skeleton_minion.glb", ANIM_IDLE);
    commands.spawn((
        scene,
        anim,
        Transform::from_translation(on_floor(4, 6, 1))
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI))
            .with_scale(Vec3::splat(SKELETON_SCALE)),
    ));

    // Skeleton warrior waiting in the floor-0 room under the pit (met by the
    // `fall` scene), combat idle, facing west toward where the player lands.
    let (scene, anim) = animated("models/enemies/skeleton_warrior.glb", ANIM_IDLE_COMBAT);
    commands.spawn((
        scene,
        anim,
        // KayKit models face +Z; rotating -90° turns them West (-X).
        Transform::from_translation(on_floor(7, 3, 0))
            .with_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(SKELETON_SCALE)),
    ));

    // The chest and barrel that used to live here are now data-driven item
    // placements (plan5, floor_items.rs); the skeletons stay hardcoded until
    // monster placement arrives in plan6.
}

/// When a glTF scene finishes spawning, its `AnimationPlayer` appears on a
/// descendant entity. Walk up to the `SceneRoot` carrying `PropAnimation` and
/// start that clip, looping.
pub fn attach_prop_animations(
    mut commands: Commands,
    mut players: Query<(Entity, &mut AnimationPlayer), Added<AnimationPlayer>>,
    parents: Query<&Parent>,
    props: Query<&PropAnimation>,
) {
    for (entity, mut player) in &mut players {
        let mut current = entity;
        loop {
            if let Ok(prop) = props.get(current) {
                commands
                    .entity(entity)
                    .insert(AnimationGraphHandle(prop.graph.clone()));
                player.play(prop.node).repeat();
                break;
            }
            match parents.get(current) {
                Ok(parent) => current = parent.get(),
                Err(_) => break,
            }
        }
    }
}
