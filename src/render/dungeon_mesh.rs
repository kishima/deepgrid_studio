use std::collections::HashSet;

use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::prelude::*;

use crate::dungeon::level::Level;
use crate::dungeon::{Block, DoorStates, Dungeon};
use crate::world::LevelScoped;

/// World size of one grid block (1.0 × 1.0 × 1.0). Tile `(x, y)` on floor `f`
/// occupies `[x, x+1] × [f, f+1] × [y, y+1]`, so its floor surface is at
/// `y = f * BLOCK_SIZE` and its center at `(x+0.5, f+0.5, y+0.5)`.
pub const BLOCK_SIZE: f32 = 1.0;

/// A door fill panel, tagged with its door kind so [`update_door_visibility`]
/// can hide it when the kind is open.
#[derive(Component)]
pub struct DoorTile {
    pub kind: u8,
}

/// Which grid cell a spawned tile entity belongs to, so [`rebuild_dirty_tiles`]
/// can despawn+respawn exactly the cells a SetBlock/SetLiquid touched (plan8).
#[derive(Component, Clone, Copy)]
pub struct TilePos {
    pub x: i32,
    pub y: i32,
    pub floor: usize,
}

/// Request to rebuild the geometry of a cell (and its 6 neighbours, whose wall
/// culling may change) after the dungeon data at `(x, y, floor)` changed.
#[derive(Event, Clone, Copy)]
pub struct TileDirty {
    pub x: i32,
    pub y: i32,
    pub floor: usize,
}

/// Meshes + materials built once and shared across every spawned tile (a Bevy
/// resource so the incremental rebuild can reuse them).
#[derive(Resource, Clone)]
pub struct Palette {
    floor_tile: Handle<Mesh>,
    wall_cube: Handle<Mesh>,
    ladder_board_x: Handle<Mesh>,
    ladder_board_z: Handle<Mesh>,
    liquid_slab: Handle<Mesh>,
    door_fill: Handle<Mesh>,
    door_lintel: Handle<Mesh>,
    horoscope_body: Handle<Mesh>,
    horoscope_arrow: Handle<Mesh>,
    step: Handle<Mesh>,
    plate: Handle<Mesh>,
    warp_disc: Handle<Mesh>,
    trigger_cube: Handle<Mesh>,

    floor_mat: Handle<StandardMaterial>,
    wall_mat: Handle<StandardMaterial>,
    writable_mat: Handle<StandardMaterial>,
    ladder_mat: Handle<StandardMaterial>,
    water_mat: Handle<StandardMaterial>,
    fire_mat: Handle<StandardMaterial>,
    poison_mat: Handle<StandardMaterial>,
    horoscope_mat: Handle<StandardMaterial>,
    vert_horo_mat: Handle<StandardMaterial>,
    arrow_mat: Handle<StandardMaterial>,
    stairs_mat: Handle<StandardMaterial>,
    plate_mat: Handle<StandardMaterial>,
    warp_mat: Handle<StandardMaterial>,
    keyhole_mat: Handle<StandardMaterial>,
    switch_mat: Handle<StandardMaterial>,
    door_fill_mats: [Handle<StandardMaterial>; 2],
    door_lintel_mats: [Handle<StandardMaterial>; 2],
    /// Ceiling texture (resolved against the project override dir, plan10);
    /// the ceiling material itself is per-level (uv scale = map size).
    ceiling_tex: Handle<Image>,
}

/// Build the static dungeon geometry across all floors (plan2 Step 2; per-cell
/// since plan8 so cells can be rebuilt individually). Cell entities are tagged
/// with [`TilePos`] + [`LevelScoped`]; the ceiling slab / ambient light are not.
pub fn setup_dungeon(
    mut commands: Commands,
    dungeon: Res<Dungeon>,
    asset_server: Res<AssetServer>,
    resolver: Res<crate::project::AssetResolver>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let palette = build_palette(&asset_server, &resolver, &mut meshes, &mut materials);
    spawn_level_mesh(&mut commands, &palette, &mut meshes, &mut materials, &dungeon.level);
    commands.insert_resource(palette);

    // High ambient base so the whole scene stays readable and the player light
    // (movement.rs) is a soft accent — a uniform, Grimrock-ish look.
    commands.insert_resource(AmbientLight {
        color: Color::srgb(0.7, 0.75, 0.9),
        brightness: 700.0,
    });
}

/// Spawn all `LevelScoped` geometry (per-cell tiles + ceiling) for `level`.
/// Shared by startup and level transitions (plan8); the palette is reused.
pub fn spawn_level_mesh(
    commands: &mut Commands,
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    level: &Level,
) {
    let floor0 = level.floor(0).expect("at least one floor");
    let (w, h) = (floor0.width, floor0.height);
    for f in 0..level.floor_count() {
        let floor = level.floor(f).expect("floor in range");
        for y in 0..floor.height {
            for x in 0..floor.width {
                spawn_cell(commands, palette, level, x as i32, y as i32, f);
            }
        }
    }
    spawn_ceiling(commands, palette, meshes, materials, w, h, level.floor_count());
}

/// Spawn the single ceiling slab over the whole map at the top floor's head.
fn spawn_ceiling(
    commands: &mut Commands,
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    w: usize,
    h: usize,
    floor_count: usize,
) {
    let top = floor_count as f32 * BLOCK_SIZE;
    let slab = meshes.add(Cuboid::new(w as f32 * BLOCK_SIZE, 0.02, h as f32 * BLOCK_SIZE));
    let ceiling_mat = materials.add(StandardMaterial {
        base_color_texture: Some(palette.ceiling_tex.clone()),
        perceptual_roughness: 0.95,
        uv_transform: Affine2::from_scale(Vec2::new(w as f32, h as f32)),
        ..default()
    });
    commands.spawn((
        Mesh3d(slab),
        MeshMaterial3d(ceiling_mat),
        Transform::from_xyz(w as f32 * BLOCK_SIZE / 2.0, top + 0.01, h as f32 * BLOCK_SIZE / 2.0),
        LevelScoped,
    ));
}

/// Spawn every entity for cell `(x, y, f)` from the current level data. Each
/// carries `TilePos` + `LevelScoped`.
fn spawn_cell(commands: &mut Commands, p: &Palette, level: &Level, x: i32, y: i32, f: usize) {
    let block = level.floor(f).and_then(|fl| fl.get(x, y)).unwrap_or_default();
    let yb = f as f32 * BLOCK_SIZE;
    let (cx, cz) = (x as f32 + 0.5, y as f32 + 0.5);
    let tile = TilePos { x, y, floor: f };

    // Solid blocks (walls / writable walls): a cube where a face is exposed.
    if block.is_solid() {
        if wall_visible(level, x, y, f) {
            let mat = if matches!(block, Block::WritableWall) {
                p.writable_mat.clone()
            } else {
                p.wall_mat.clone()
            };
            commands.spawn((
                Mesh3d(p.wall_cube.clone()),
                MeshMaterial3d(mat),
                Transform::from_xyz(cx, yb + BLOCK_SIZE / 2.0, cz),
                tile,
                LevelScoped,
            ));
        }
        return;
    }

    // Front-facing trigger blocks (keyhole / switch) are short pillars.
    if matches!(block, Block::Keyhole | Block::Switch) {
        let mat = if matches!(block, Block::Keyhole) { p.keyhole_mat.clone() } else { p.switch_mat.clone() };
        commands.spawn((
            Mesh3d(p.trigger_cube.clone()),
            MeshMaterial3d(mat),
            Transform::from_xyz(cx, yb + 0.45, cz),
            tile,
            LevelScoped,
        ));
        return;
    }

    // Non-solid cell: a floor surface where it has footing from below (a solid
    // block beneath, or bedrock) — but never under a hole.
    let below_solid = level.floor(f.wrapping_sub(1)).and_then(|b| b.get(x, y)).is_some_and(|b| b.is_solid());
    if !block.is_hole() && (f == 0 || below_solid) {
        // Lift the tile a hair off the supporting cube's top face — coplanar
        // quads z-fight (same trick as the ceiling slab).
        let lift = if below_solid { 0.01 } else { 0.0 };
        commands.spawn((
            Mesh3d(p.floor_tile.clone()),
            MeshMaterial3d(p.floor_mat.clone()),
            Transform::from_xyz(cx, yb + lift, cz),
            tile,
            LevelScoped,
        ));
    }

    spawn_decor(commands, p, block, tile, cx, yb, cz);
}

/// Hide a door's fill panel when its kind is open, show it when closed.
pub fn update_door_visibility(doors: Res<DoorStates>, mut tiles: Query<(&DoorTile, &mut Visibility)>) {
    for (tile, mut vis) in &mut tiles {
        *vis = if doors.is_open(tile.kind) { Visibility::Hidden } else { Visibility::Visible };
    }
}

/// Rebuild the geometry of every cell touched by a `TileDirty`, plus its six
/// neighbours (whose wall culling may flip). Runs after SetBlock/SetLiquid have
/// updated the `Dungeon` data.
pub fn rebuild_dirty_tiles(
    mut commands: Commands,
    mut dirty: EventReader<TileDirty>,
    palette: Option<Res<Palette>>,
    dungeon: Res<Dungeon>,
    tiles: Query<(Entity, &TilePos)>,
) {
    let Some(palette) = palette else { return };
    let mut cells: HashSet<(i32, i32, usize)> = HashSet::new();
    let fc = dungeon.level.floor_count() as i32;
    for ev in dirty.read() {
        let f = ev.floor as i32;
        for (dx, dy, df) in [(0, 0, 0), (1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let nf = f + df;
            if nf < 0 || nf >= fc {
                continue;
            }
            cells.insert((ev.x + dx, ev.y + dy, nf as usize));
        }
    }
    if cells.is_empty() {
        return;
    }
    for (e, tp) in &tiles {
        if cells.contains(&(tp.x, tp.y, tp.floor)) {
            commands.entity(e).despawn_recursive();
        }
    }
    for (x, y, f) in cells {
        spawn_cell(&mut commands, &palette, &dungeon.level, x, y, f);
    }
}

fn wall_visible(level: &Level, x: i32, y: i32, f: usize) -> bool {
    let open = |xx: i32, yy: i32, ff: i32| -> bool {
        if ff < 0 || ff as usize >= level.floor_count() {
            return false;
        }
        match level.floor(ff as usize).and_then(|b| b.get(xx, yy)) {
            None => false,
            Some(b) => !b.is_solid(),
        }
    };
    let fi = f as i32;
    open(x - 1, y, fi)
        || open(x + 1, y, fi)
        || open(x, y - 1, fi)
        || open(x, y + 1, fi)
        || open(x, y, fi + 1)
        || open(x, y, fi - 1)
}

/// Spawn one decoration mesh tagged with its tile + level scope.
fn deco(commands: &mut Commands, mesh: &Handle<Mesh>, mat: &Handle<StandardMaterial>, tile: TilePos, tf: Transform) {
    commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(mat.clone()), tf, tile, LevelScoped));
}

/// Spawn the block-specific decoration for a non-wall, non-trigger cell.
fn spawn_decor(commands: &mut Commands, p: &Palette, block: Block, tile: TilePos, cx: f32, yb: f32, cz: f32) {
    match block {
        Block::Empty | Block::Wall | Block::WritableWall | Block::Keyhole | Block::Switch | Block::Hole => {}
        Block::Ladder => {
            let y = yb + BLOCK_SIZE / 2.0;
            deco(commands, &p.ladder_board_x, &p.ladder_mat, tile, Transform::from_xyz(cx, y, cz));
            deco(commands, &p.ladder_board_z, &p.ladder_mat, tile, Transform::from_xyz(cx, y, cz));
        }
        Block::Water => deco(commands, &p.liquid_slab, &p.water_mat, tile, Transform::from_xyz(cx, yb + 0.03, cz)),
        Block::Fire => deco(commands, &p.liquid_slab, &p.fire_mat, tile, Transform::from_xyz(cx, yb + 0.03, cz)),
        Block::Poison => deco(commands, &p.liquid_slab, &p.poison_mat, tile, Transform::from_xyz(cx, yb + 0.03, cz)),
        Block::Door { kind } => {
            let idx = (kind as usize).min(1);
            commands.spawn((
                Mesh3d(p.door_fill.clone()),
                MeshMaterial3d(p.door_fill_mats[idx].clone()),
                Transform::from_xyz(cx, yb + 0.41, cz),
                DoorTile { kind },
                tile,
                LevelScoped,
            ));
            deco(commands, &p.door_lintel, &p.door_lintel_mats[idx], tile, Transform::from_xyz(cx, yb + 0.92, cz));
        }
        Block::Horoscope { pass_from } => {
            deco(commands, &p.horoscope_body, &p.horoscope_mat, tile, Transform::from_xyz(cx, yb + 0.4, cz));
            let (dx, dy) = pass_from.delta();
            deco(commands, &p.horoscope_arrow, &p.arrow_mat, tile, Transform::from_xyz(cx + dx as f32 * 0.42, yb + 0.4, cz + dy as f32 * 0.42));
        }
        Block::HoroscopeVert { from_below } => {
            deco(commands, &p.horoscope_body, &p.vert_horo_mat, tile, Transform::from_xyz(cx, yb + 0.4, cz));
            let ny = if from_below { yb + 0.85 } else { yb };
            deco(commands, &p.horoscope_arrow, &p.arrow_mat, tile, Transform::from_xyz(cx, ny, cz));
        }
        Block::Stairs { up } => {
            deco(commands, &p.step, &p.stairs_mat, tile, Transform::from_xyz(cx, yb + 0.1, cz + if up { 0.2 } else { -0.2 }));
            deco(commands, &p.step, &p.stairs_mat, tile, Transform::from_xyz(cx, yb + 0.3, cz + if up { -0.05 } else { 0.05 }));
        }
        Block::FloorPlate => deco(commands, &p.plate, &p.plate_mat, tile, Transform::from_xyz(cx, yb + 0.04, cz)),
        Block::WarpPoint => deco(commands, &p.warp_disc, &p.warp_mat, tile, Transform::from_xyz(cx, yb + 0.2, cz)),
    }
}

/// Build the shared tile palette (meshes + materials). Public so the 3D editor
/// (plan9.5) can build its own scene without a play-mode `Dungeon`. Terrain
/// textures resolve through the project's `override/` directory (plan10).
pub fn build_palette(
    asset_server: &AssetServer,
    resolver: &crate::project::AssetResolver,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Palette {
    let floor_tile = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(BLOCK_SIZE / 2.0)));
    let wall_cube = meshes.add(wall_cube_mesh());
    let ladder_board_x = meshes.add(Cuboid::new(0.7, BLOCK_SIZE, 0.08));
    let ladder_board_z = meshes.add(Cuboid::new(0.08, BLOCK_SIZE, 0.7));
    let liquid_slab = meshes.add(Cuboid::new(0.94, 0.02, 0.94));
    let door_fill = meshes.add(Cuboid::new(0.82, 0.82, 0.82));
    let door_lintel = meshes.add(Cuboid::new(0.92, 0.16, 0.92));
    let horoscope_body = meshes.add(Cuboid::new(0.8, 0.8, 0.8));
    let horoscope_arrow = meshes.add(Cuboid::new(0.2, 0.2, 0.2));
    let step = meshes.add(Cuboid::new(0.8, 0.2, 0.4));
    let plate = meshes.add(Cuboid::new(0.82, 0.08, 0.82));
    let warp_disc = meshes.add(Cuboid::new(0.7, 0.04, 0.7));
    let trigger_cube = meshes.add(Cuboid::new(0.7, 0.9, 0.7));

    let floor_mat = materials.add(StandardMaterial {
        base_color_texture: Some(load_repeating(asset_server, resolver.resolve("textures/floor_pavingstones119_color.png"))),
        perceptual_roughness: 0.95,
        ..default()
    });
    let wall_mat = materials.add(StandardMaterial {
        base_color_texture: Some(load_repeating(asset_server, resolver.resolve("textures/wall_bricks066_color.png"))),
        perceptual_roughness: 0.9,
        ..default()
    });
    let ceiling_tex = load_repeating(asset_server, resolver.resolve("textures/ceiling_rock058_color.png"));
    let writable_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.50, 0.42),
        emissive: LinearRgba::rgb(0.10, 0.09, 0.05),
        perceptual_roughness: 0.9,
        ..default()
    });
    let ladder_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.30, 0.15),
        perceptual_roughness: 0.8,
        ..default()
    });
    let liquid = |c: Color| StandardMaterial {
        base_color: c,
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.3,
        ..default()
    };
    let water_mat = materials.add(liquid(Color::srgba(0.20, 0.45, 0.95, 0.5)));
    let fire_mat = materials.add(liquid(Color::srgba(0.95, 0.45, 0.12, 0.55)));
    let poison_mat = materials.add(liquid(Color::srgba(0.35, 0.85, 0.25, 0.5)));
    let horoscope_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.60, 0.25, 0.85, 0.30),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let vert_horo_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.30, 0.55, 0.90, 0.30),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let arrow_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.9, 0.2),
        emissive: LinearRgba::rgb(0.6, 0.5, 0.05),
        ..default()
    });
    let stairs_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.42, 0.25),
        perceptual_roughness: 0.85,
        ..default()
    });
    let plate_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.48, 0.55),
        perceptual_roughness: 0.7,
        ..default()
    });
    let warp_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.4, 0.9, 0.95),
        emissive: LinearRgba::rgb(0.15, 0.6, 0.7),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let keyhole_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.48, 0.18),
        perceptual_roughness: 0.6,
        ..default()
    });
    let switch_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.65, 0.22, 0.22),
        perceptual_roughness: 0.6,
        ..default()
    });
    let door_fill_mats = [
        materials.add(StandardMaterial { base_color: Color::srgb(0.85, 0.65, 0.25), perceptual_roughness: 0.6, ..default() }),
        materials.add(StandardMaterial { base_color: Color::srgb(0.30, 0.70, 0.75), perceptual_roughness: 0.6, ..default() }),
    ];
    let door_lintel_mats = [
        materials.add(StandardMaterial { base_color: Color::srgb(0.50, 0.38, 0.15), ..default() }),
        materials.add(StandardMaterial { base_color: Color::srgb(0.18, 0.42, 0.45), ..default() }),
    ];

    Palette {
        floor_tile,
        wall_cube,
        ladder_board_x,
        ladder_board_z,
        liquid_slab,
        door_fill,
        door_lintel,
        horoscope_body,
        horoscope_arrow,
        step,
        plate,
        warp_disc,
        trigger_cube,
        floor_mat,
        wall_mat,
        writable_mat,
        ladder_mat,
        water_mat,
        fire_mat,
        poison_mat,
        horoscope_mat,
        vert_horo_mat,
        arrow_mat,
        stairs_mat,
        plate_mat,
        warp_mat,
        keyhole_mat,
        switch_mat,
        door_fill_mats,
        door_lintel_mats,
        ceiling_tex,
    }
}

/// A unit cube whose UVs are rebuilt so the texture reads upright on every side
/// face.
fn wall_cube_mesh() -> Mesh {
    let mut mesh = Mesh::from(Cuboid::new(BLOCK_SIZE, BLOCK_SIZE, BLOCK_SIZE));
    let positions: Vec<[f32; 3]> = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|a| a.as_float3())
        .expect("cuboid has positions")
        .to_vec();
    let normals: Vec<[f32; 3]> = mesh
        .attribute(Mesh::ATTRIBUTE_NORMAL)
        .and_then(|a| a.as_float3())
        .expect("cuboid has normals")
        .to_vec();
    let uvs: Vec<[f32; 2]> = positions
        .iter()
        .zip(&normals)
        .map(|(pos, n)| {
            if n[0].abs() > 0.5 {
                [pos[2] + 0.5, 0.5 - pos[1]]
            } else if n[2].abs() > 0.5 {
                [pos[0] + 0.5, 0.5 - pos[1]]
            } else {
                [pos[0] + 0.5, pos[2] + 0.5]
            }
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh
}

/// Load a texture with repeat wrapping so tiling works.
fn load_repeating(asset_server: &AssetServer, path: String) -> Handle<Image> {
    asset_server.load_with_settings(path, |settings: &mut ImageLoaderSettings| {
        settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            ..ImageSamplerDescriptor::default()
        });
    })
}
