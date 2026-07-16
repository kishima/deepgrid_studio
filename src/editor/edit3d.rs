//! 3D edit mode (plan9.5): walk the dungeon in first person and place / erase on
//! the tile ahead. All edits go through the existing `EditorState` methods (the
//! same path as the 2D grid), so Undo/Redo, dirty tracking and the warning panel
//! stay unified. The 3D scene (camera + mesh + placement markers) is tagged
//! `Edit3dScoped` and torn down on return to 2D.

use bevy::pbr::ClusterConfig;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_egui::EguiContexts;

use super::{EditorState, PlaceLayer};
use super::walk::{EditWalk, WalkAction};
use crate::dungeon::{Dungeon, Facing, GridPos};
use crate::player::movement::FOV_DEGREES;
use crate::render::{BLOCK_SIZE, Palette, TileDirty, spawn_level_mesh};
use crate::settings::Keybinds;
use crate::world::LevelScoped;

/// Marks every entity of the live 3D-edit scene (camera, light, mesh, markers).
#[derive(Component)]
pub struct Edit3dScoped;

/// Marks a placement marker (item / monster / event) so they can be respawned
/// without touching the terrain mesh.
#[derive(Component)]
pub struct Edit3dMarker;

/// The 3D-edit camera.
#[derive(Component)]
pub struct Edit3dCamera;

/// Whether the 3D scene is currently spawned.
#[derive(Resource, Default)]
pub struct Edit3dSpawned(pub bool);

/// Register the 3D-edit resources + systems on the unified App (plan13). Runs
/// only in [`GameScreen::Editor`]; `Keybinds` / `TileDirty` already exist on the
/// play App, so only the edit3d-specific resources are added here.
pub fn register(app: &mut App) {
    use crate::screen::GameScreen;
    app.init_resource::<Edit3dSpawned>()
        .insert_resource(EditWalk::new(GridPos::new(0, 0, 0), Facing::North))
        .add_systems(
            Update,
            (
                edit3d_manage,
                edit3d_sync.after(edit3d_manage),
                crate::render::rebuild_dirty_tiles.after(edit3d_sync),
                edit3d_walk.after(edit3d_manage),
                edit3d_camera.after(edit3d_walk),
                edit3d_place.after(edit3d_manage),
            )
                .run_if(in_state(GameScreen::Editor)),
        );
}

/// Enter / leave 3D mode: spawn or despawn the scene, toggle the 2D camera.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn edit3d_manage(
    mut commands: Commands,
    mut state: ResMut<EditorState>,
    mut spawned: ResMut<Edit3dSpawned>,
    mut walk: ResMut<EditWalk>,
    scoped: Query<Entity, Or<(With<Edit3dScoped>, With<LevelScoped>)>>,
    mut cam2d: Query<&mut Camera, With<Camera2d>>,
) {
    if state.mode_3d && !spawned.0 {
        // Enter: start the walker at the level's start, disable the 2D camera,
        // spawn the first-person camera + headlamp, and request a full build.
        *walk = EditWalk::new(state.cur().start, state.cur().start_facing);
        state.floor_index = walk.pos.floor;
        for mut c in &mut cam2d {
            c.is_active = false;
        }
        let (eye, yaw) = walk.camera_target();
        commands
            .spawn((
                Camera3d::default(),
                Msaa::Off,
                ClusterConfig::Single,
                Projection::Perspective(PerspectiveProjection { fov: FOV_DEGREES.to_radians(), ..default() }),
                Transform::from_translation(eye).with_rotation(Quat::from_rotation_y(yaw)),
                Edit3dScoped,
                Edit3dCamera,
            ))
            .with_children(|p| {
                p.spawn((
                    PointLight {
                        intensity: crate::magic::BASE_LIGHT_INTENSITY,
                        range: crate::magic::BASE_LIGHT_RANGE,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_xyz(0.0, 0.3, 0.0),
                    Edit3dScoped,
                ));
            });
        commands.insert_resource(AmbientLight { color: Color::srgb(0.7, 0.75, 0.9), brightness: 700.0 });
        state.d3_full = true;
        spawned.0 = true;
    } else if !state.mode_3d && spawned.0 {
        // Leave: despawn the whole 3D scene and re-enable the 2D camera.
        for e in &scoped {
            commands.entity(e).despawn_recursive();
        }
        for mut c in &mut cam2d {
            c.is_active = true;
        }
        state.d3_terrain_dirty.clear();
        state.d3_markers_dirty = false;
        state.d3_full = false;
        spawned.0 = false;
    }
}

/// Reflect edits into the 3D scene: full rebuild on level/undo changes, else per-
/// cell `TileDirty` for terrain + marker respawn. When 2D, just drain the flags.
#[allow(clippy::too_many_arguments)]
fn edit3d_sync(
    mut commands: Commands,
    mut state: ResMut<EditorState>,
    spawned: Res<Edit3dSpawned>,
    mut dungeon: ResMut<Dungeon>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    palette: Option<Res<Palette>>,
    mut tile_dirty: EventWriter<TileDirty>,
    markers: Query<Entity, With<Edit3dMarker>>,
    mesh_tiles: Query<Entity, With<LevelScoped>>,
) {
    if !spawned.0 {
        // 2D mode: don't let edit flags accumulate.
        state.d3_terrain_dirty.clear();
        state.d3_markers_dirty = false;
        state.d3_full = false;
        return;
    }
    let Some(palette) = palette else { return };

    if state.d3_full {
        // Rebuild everything: sync the Dungeon mirror, respawn mesh + markers.
        dungeon.level = state.cur().level.clone();
        for e in &mesh_tiles {
            commands.entity(e).despawn_recursive();
        }
        spawn_level_mesh(&mut commands, &palette, &mut meshes, &mut materials, &dungeon.level);
        respawn_markers(&mut commands, &state, &asset_server, &mut meshes, &mut materials, &markers);
        state.d3_terrain_dirty.clear();
        state.d3_markers_dirty = false;
        state.d3_full = false;
        return;
    }

    // Per-cell terrain edits: mirror into the Dungeon and mark dirty.
    if !state.d3_terrain_dirty.is_empty() {
        let cells: Vec<GridPos> = std::mem::take(&mut state.d3_terrain_dirty);
        for pos in cells {
            if let Some(b) = state.cur().level.block_at(pos) {
                dungeon.level.set_block(pos, b);
                tile_dirty.send(TileDirty { x: pos.x, y: pos.y, floor: pos.floor });
            }
        }
    }
    if state.d3_markers_dirty {
        respawn_markers(&mut commands, &state, &asset_server, &mut meshes, &mut materials, &markers);
        state.d3_markers_dirty = false;
    }
}

/// KayKit humanoid scale (matches play-mode monsters).
const MARKER_MONSTER_SCALE: f32 = 0.45;

/// Despawn and respawn all item / monster / event markers for the current level.
fn respawn_markers(
    commands: &mut Commands,
    state: &EditorState,
    asset_server: &AssetServer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    markers: &Query<Entity, With<Edit3dMarker>>,
) {
    for e in markers {
        commands.entity(e).despawn_recursive();
    }
    let center = |p: GridPos, dy: f32| {
        Vec3::new(
            p.x as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
            p.floor as f32 * BLOCK_SIZE + dy,
            p.y as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
        )
    };
    let lvl = state.cur();

    // Items: the glb if the def names a model, else a tinted gem.
    let gem = meshes.add(Cuboid::new(0.18, 0.18, 0.18));
    for p in &lvl.items {
        let pos = GridPos::new(p.x, p.y, p.floor);
        let def = state.proj.items.iter().find(|d| d.id == p.id);
        match def {
            Some(d) if !d.model.is_empty() => {
                commands.spawn((
                    SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(d.model.clone()))),
                    Transform::from_translation(center(pos, 0.0)).with_scale(Vec3::splat(0.3)),
                    Edit3dScoped,
                    Edit3dMarker,
                ));
            }
            _ => {
                let (r, g, b) = def.map(|d| d.kind.color()).unwrap_or((0.8, 0.8, 0.4));
                let mat = materials.add(StandardMaterial {
                    base_color: Color::srgb(r, g, b),
                    emissive: LinearRgba::rgb(r * 0.4, g * 0.4, b * 0.4),
                    ..default()
                });
                commands.spawn((
                    Mesh3d(gem.clone()),
                    MeshMaterial3d(mat),
                    Transform::from_translation(center(pos, 0.45)),
                    Edit3dScoped,
                    Edit3dMarker,
                ));
            }
        }
    }

    // Monsters: a static (non-animated) glb bust.
    for p in &lvl.monsters {
        let pos = GridPos::new(p.x, p.y, p.floor);
        if let Some(d) = state.proj.monsters.iter().find(|d| d.id == p.id) {
            commands.spawn((
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(d.model.clone()))),
                Transform::from_translation(center(pos, 0.0)).with_scale(Vec3::splat(MARKER_MONSTER_SCALE)),
                Edit3dScoped,
                Edit3dMarker,
            ));
        }
    }

    // Events: a translucent marker at each event's coordinate — including hidden
    // warps / triggers, which must be visible while editing.
    let ev_mesh = meshes.add(Cuboid::new(0.5, 0.5, 0.5));
    for ev in &lvl.events {
        let (ex, ey, ef) = ev.at;
        let color = trigger_color(&ev.trigger);
        let mat = materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::new(color.to_srgba().red * 0.5, color.to_srgba().green * 0.5, color.to_srgba().blue * 0.5, 1.0),
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        commands.spawn((
            Mesh3d(ev_mesh.clone()),
            MeshMaterial3d(mat),
            Transform::from_translation(center(GridPos::new(ex, ey, ef), 0.6)),
            Edit3dScoped,
            Edit3dMarker,
        ));
    }
}

fn trigger_color(t: &crate::event::TriggerKind) -> Color {
    use crate::event::TriggerKind::*;
    match t {
        Keyhole { .. } => Color::srgba(0.9, 0.85, 0.2, 0.6),
        SwitchOneWay | SwitchToggle | SwitchPush => Color::srgba(0.9, 0.3, 0.3, 0.6),
        FloorPlate { .. } => Color::srgba(0.55, 0.6, 0.75, 0.6),
        WarpPoint { .. } => Color::srgba(0.3, 0.9, 0.95, 0.6),
        None => Color::srgba(0.6, 0.6, 0.6, 0.5),
    }
}

/// Keyboard walk (guarded so typing in an egui field doesn't move the camera).
fn edit3d_walk(
    spawned: Res<Edit3dSpawned>,
    keys: Res<ButtonInput<KeyCode>>,
    keybinds: Res<Keybinds>,
    mut contexts: EguiContexts,
    mut walk: ResMut<EditWalk>,
    mut state: ResMut<EditorState>,
    mut last_level: Local<Option<usize>>,
) {
    if !spawned.0 {
        *last_level = None;
        return;
    }
    // Level switched (or the level under our feet vanished — delete/undo of a
    // resize): restart at the level's start. Otherwise follow the egui floor
    // selector: this system is the only writer of `floor_index` while walking,
    // so any mismatch seen here is a user pick in the combo.
    if *last_level != Some(state.level_index)
        || state.cur().level.block_at(walk.pos).is_none()
    {
        *walk = EditWalk::new(state.cur().start, state.cur().start_facing);
        *last_level = Some(state.level_index);
        state.floor_index = walk.pos.floor;
    } else if state.floor_index != walk.pos.floor {
        let dest = GridPos::new(walk.pos.x, walk.pos.y, state.floor_index);
        if EditWalk::passable(&state.cur().level, dest) {
            walk.pos.floor = state.floor_index;
        } else {
            // A wall occupies this cell on the picked floor: refuse the jump
            // (the combo snaps back to the walker's floor).
            state.floor_index = walk.pos.floor;
        }
    }
    if contexts.ctx_mut().wants_keyboard_input() {
        return;
    }
    let level = state.cur().level.clone();
    // Reuse the play keybinds for movement direction; map to editor walk actions.
    use crate::player::Command;
    let action = keybinds.command_for(|k| keys.just_pressed(k)).and_then(|c| match c {
        Command::Move(crate::player::Action::Forward) => Some(WalkAction::Forward),
        Command::Move(crate::player::Action::Backward) => Some(WalkAction::Backward),
        Command::Move(crate::player::Action::StrafeLeft) => Some(WalkAction::StrafeLeft),
        Command::Move(crate::player::Action::StrafeRight) => Some(WalkAction::StrafeRight),
        Command::Move(crate::player::Action::TurnLeft) => Some(WalkAction::TurnLeft),
        Command::Move(crate::player::Action::TurnRight) => Some(WalkAction::TurnRight),
        _ => None,
    });
    if let Some(a) = action {
        walk.step(a, &level);
    }
    if keys.just_pressed(KeyCode::KeyR) {
        walk.climb(true, &level);
    }
    if keys.just_pressed(KeyCode::KeyF) {
        walk.climb(false, &level);
    }
    // Keep the egui floor selector in step with the walker.
    if state.floor_index != walk.pos.floor {
        state.floor_index = walk.pos.floor;
    }
    let dir = match walk.facing {
        Facing::North => "N",
        Facing::East => "E",
        Facing::South => "S",
        Facing::West => "W",
    };
    state.d3_coord = format!("({}, {}, F{}) {dir}", walk.pos.x, walk.pos.y, walk.pos.floor);
}

/// Smoothly ease the camera toward the walker's canonical pose.
fn edit3d_camera(
    spawned: Res<Edit3dSpawned>,
    time: Res<Time>,
    walk: Res<EditWalk>,
    mut cam: Query<&mut Transform, With<Edit3dCamera>>,
) {
    if !spawned.0 {
        return;
    }
    let (eye, yaw) = walk.camera_target();
    let target_rot = Quat::from_rotation_y(yaw);
    let k = (time.delta_secs() / 0.25 * 2.0).min(1.0);
    for mut tf in &mut cam {
        tf.translation = tf.translation.lerp(eye, k);
        tf.rotation = tf.rotation.slerp(target_rot, k);
    }
}

/// Left-click = place the selected palette part on the tile ahead; right-click =
/// erase it. Guarded so clicks on the egui panels don't edit.
fn edit3d_place(
    spawned: Res<Edit3dSpawned>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut contexts: EguiContexts,
    walk: Res<EditWalk>,
    mut state: ResMut<EditorState>,
) {
    if !spawned.0 {
        return;
    }
    let (place, erase) = (mouse.just_pressed(MouseButton::Left), mouse.just_pressed(MouseButton::Right));
    if !place && !erase {
        return;
    }
    if contexts.ctx_mut().wants_pointer_input() {
        return;
    }
    let front = walk.front();
    // Edits use floor_index; align it to the tile ahead (same floor as the walker).
    state.floor_index = front.floor;
    // Bounds check against the current level.
    let (w, h) = (state.cur().width() as i32, state.cur().height() as i32);
    if front.x < 0 || front.y < 0 || front.x >= w || front.y >= h {
        return;
    }
    if place {
        // Block layer must finalise its stroke (place_at → paint uses the stroke).
        state.place_at(front.x, front.y);
        if state.place_layer == PlaceLayer::Block {
            state.end_stroke();
        }
    } else {
        state.erase_at(front.x, front.y);
    }
}

// ------------------------------------------------------------------ screenshot

/// Force 3D mode, position a viewpoint, then Bevy-screenshot the 3D view
/// (`DEEPGRID_DEBUG_SHOT=editor-3d`). egui panels aren't captured by the Bevy
/// screenshot path (a known limitation); the egui editor is covered by the
/// `editor-map` shot.
pub fn edit3d_shot_driver(
    mut frames: Local<u32>,
    mut shot_frame: Local<Option<u32>>,
    mut state: ResMut<EditorState>,
    mut walk: ResMut<EditWalk>,
    mut commands: Commands,
    mut exit: EventWriter<AppExit>,
) {
    *frames += 1;
    if *frames == 5 {
        state.mode_3d = true; // enter 3D (edit3d_manage handles the scene)
    }
    if *frames == 25 {
        // Look south across the floor-1 item room so markers are in frame.
        *walk = EditWalk::new(GridPos::new(6, 4, 1), Facing::South);
        state.floor_index = 1;
    }
    match *shot_frame {
        None => {
            if *frames >= 70 {
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk("debug-shot.png"));
                *shot_frame = Some(*frames);
            }
        }
        Some(f) => {
            if *frames >= f + 15 {
                exit.send(AppExit::Success);
            }
        }
    }
}
