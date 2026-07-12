use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::prelude::*;

use crate::dungeon::level::Level;
use crate::dungeon::{Block, DoorStates, Dungeon, Facing};

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

/// Meshes + materials built once and shared across every spawned tile.
struct Palette {
    floor_tile: Handle<Mesh>,
    wall_cube: Handle<Mesh>,
    ladder_board_x: Handle<Mesh>,
    ladder_board_z: Handle<Mesh>,
    liquid_slab: Handle<Mesh>,
    door_fill: Handle<Mesh>,
    door_lintel: Handle<Mesh>,
    horoscope_body: Handle<Mesh>,
    horoscope_arrow: Handle<Mesh>,

    floor_mat: Handle<StandardMaterial>,
    wall_mat: Handle<StandardMaterial>,
    ladder_mat: Handle<StandardMaterial>,
    water_mat: Handle<StandardMaterial>,
    fire_mat: Handle<StandardMaterial>,
    poison_mat: Handle<StandardMaterial>,
    horoscope_mat: Handle<StandardMaterial>,
    arrow_mat: Handle<StandardMaterial>,
    door_fill_mats: [Handle<StandardMaterial>; 2],
    door_lintel_mats: [Handle<StandardMaterial>; 2],
}

/// Build the static dungeon geometry across all floors (plan2 Step 2).
///
/// Unlike plan1's single floor+ceiling slab, floors are drawn as *per-cell*
/// tiles: a cell gets a floor tile only where it is supported by a wall below
/// (or bedrock on floor 0), so holes and open shafts read naturally. Walls,
/// ladders, doors, liquids and horoscope markers are placed per cell at their
/// floor's `y = f * BLOCK_SIZE` offset. Only the topmost floor gets a full
/// ceiling slab; every interior "ceiling" is just the underside of the walls
/// above.
///
/// Kept deliberately naive (single-color/textured materials, no face culling),
/// with one concession for the software renderer: fully buried wall cubes (no
/// open neighbour on any of their six faces) are skipped — they can never be
/// seen.
pub fn setup_dungeon(
    mut commands: Commands,
    dungeon: Res<Dungeon>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let level = &dungeon.level;
    let floor0 = level.floor(0).expect("at least one floor");
    let (w, h) = (floor0.width, floor0.height);
    let palette = build_palette(&asset_server, &mut meshes, &mut materials);

    for f in 0..level.floor_count() {
        let floor = level.floor(f).expect("floor in range");
        let yb = f as f32 * BLOCK_SIZE;
        for y in 0..floor.height {
            for x in 0..floor.width {
                let (xi, yi) = (x as i32, y as i32);
                let block = floor.get(xi, yi).unwrap_or_default();
                let (cx, cz) = (x as f32 + 0.5, y as f32 + 0.5);

                if block.is_wall() {
                    if wall_visible(level, xi, yi, f) {
                        commands.spawn((
                            Mesh3d(palette.wall_cube.clone()),
                            MeshMaterial3d(palette.wall_mat.clone()),
                            Transform::from_xyz(cx, yb + BLOCK_SIZE / 2.0, cz),
                        ));
                    }
                    continue;
                }

                // Non-wall cell: draw its floor surface where it has footing
                // from below (a wall on the floor beneath, or bedrock). This is
                // the render rule — ladders do *not* conjure a floor tile.
                if f == 0
                    || level.floor(f - 1).and_then(|b| b.get(xi, yi)) == Some(Block::Wall)
                {
                    commands.spawn((
                        Mesh3d(palette.floor_tile.clone()),
                        MeshMaterial3d(palette.floor_mat.clone()),
                        Transform::from_xyz(cx, yb, cz),
                    ));
                }

                spawn_decor(&mut commands, &palette, block, cx, yb, cz);
            }
        }
    }

    // Single ceiling slab over the whole map at the top floor's head.
    let top = level.floor_count() as f32 * BLOCK_SIZE;
    let slab = meshes.add(Cuboid::new(
        w as f32 * BLOCK_SIZE,
        0.02,
        h as f32 * BLOCK_SIZE,
    ));
    let ceiling_mat = materials.add(StandardMaterial {
        base_color_texture: Some(load_repeating(
            &asset_server,
            "textures/ceiling_rock058_color.png",
        )),
        perceptual_roughness: 0.95,
        uv_transform: Affine2::from_scale(Vec2::new(w as f32, h as f32)),
        ..default()
    });
    commands.spawn((
        Mesh3d(slab),
        MeshMaterial3d(ceiling_mat),
        Transform::from_xyz(
            w as f32 * BLOCK_SIZE / 2.0,
            top + 0.01,
            h as f32 * BLOCK_SIZE / 2.0,
        ),
    ));

    // High ambient base so the whole scene stays readable and the player light
    // (movement.rs) is a soft accent — a uniform, Grimrock-ish look.
    commands.insert_resource(AmbientLight {
        color: Color::srgb(0.7, 0.75, 0.9),
        brightness: 700.0,
    });
}

/// Hide a door's fill panel when its kind is open, show it when closed. Runs
/// every frame (there are only a handful of doors); the `DoorStates` toggle in
/// movement.rs is picked up here.
pub fn update_door_visibility(
    doors: Res<DoorStates>,
    mut tiles: Query<(&DoorTile, &mut Visibility)>,
) {
    for (tile, mut vis) in &mut tiles {
        *vis = if doors.is_open(tile.kind) {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
}

/// Whether a wall cube at `(x, y, f)` has at least one open (non-wall) neighbour
/// among its six faces, and is therefore worth drawing. Out-of-range neighbours
/// (map edge, below floor 0, above the top) count as solid: the camera never
/// sees those faces.
fn wall_visible(level: &Level, x: i32, y: i32, f: usize) -> bool {
    let open = |xx: i32, yy: i32, ff: i32| -> bool {
        if ff < 0 || ff as usize >= level.floor_count() {
            return false;
        }
        match level.floor(ff as usize).and_then(|b| b.get(xx, yy)) {
            None => false,
            Some(b) => !b.is_wall(),
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

/// Spawn the block-specific decoration for a non-wall cell.
fn spawn_decor(commands: &mut Commands, p: &Palette, block: Block, cx: f32, yb: f32, cz: f32) {
    match block {
        Block::Empty | Block::Wall => {}
        Block::Ladder => {
            // Two thin crossed boards spanning the cell height, legible from any
            // angle (a stand-in until textures arrive in plan10).
            let y = yb + BLOCK_SIZE / 2.0;
            for mesh in [&p.ladder_board_x, &p.ladder_board_z] {
                commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(p.ladder_mat.clone()),
                    Transform::from_xyz(cx, y, cz),
                ));
            }
        }
        Block::Water => spawn_liquid(commands, p, p.water_mat.clone(), cx, yb, cz),
        Block::Fire => spawn_liquid(commands, p, p.fire_mat.clone(), cx, yb, cz),
        Block::Poison => spawn_liquid(commands, p, p.poison_mat.clone(), cx, yb, cz),
        Block::Door { kind } => {
            let idx = (kind as usize).min(1);
            // Fill panel: shown when closed, hidden when open. Fills the cell so
            // a shut door blocks the view.
            commands.spawn((
                Mesh3d(p.door_fill.clone()),
                MeshMaterial3d(p.door_fill_mats[idx].clone()),
                Transform::from_xyz(cx, yb + 0.41, cz),
                DoorTile { kind },
            ));
            // Lintel/header, always visible — an open door still reads as one.
            commands.spawn((
                Mesh3d(p.door_lintel.clone()),
                MeshMaterial3d(p.door_lintel_mats[idx].clone()),
                Transform::from_xyz(cx, yb + 0.92, cz),
            ));
        }
        Block::Horoscope { pass_from } => spawn_horoscope(commands, p, pass_from, cx, yb, cz),
    }
}

fn spawn_liquid(
    commands: &mut Commands,
    p: &Palette,
    mat: Handle<StandardMaterial>,
    cx: f32,
    yb: f32,
    cz: f32,
) {
    commands.spawn((
        Mesh3d(p.liquid_slab.clone()),
        MeshMaterial3d(mat),
        Transform::from_xyz(cx, yb + 0.03, cz),
    ));
}

fn spawn_horoscope(commands: &mut Commands, p: &Palette, pass_from: Facing, cx: f32, yb: f32, cz: f32) {
    // Translucent tinted body marking the block...
    commands.spawn((
        Mesh3d(p.horoscope_body.clone()),
        MeshMaterial3d(p.horoscope_mat.clone()),
        Transform::from_xyz(cx, yb + 0.4, cz),
    ));
    // ...plus a bright nub on the face you may travel toward, so the one-way
    // direction is legible on screen.
    let (dx, dy) = pass_from.delta();
    commands.spawn((
        Mesh3d(p.horoscope_arrow.clone()),
        MeshMaterial3d(p.arrow_mat.clone()),
        Transform::from_xyz(cx + dx as f32 * 0.42, yb + 0.4, cz + dy as f32 * 0.42),
    ));
}

fn build_palette(
    asset_server: &AssetServer,
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

    let floor_mat = materials.add(StandardMaterial {
        base_color_texture: Some(load_repeating(
            asset_server,
            "textures/floor_pavingstones119_color.png",
        )),
        perceptual_roughness: 0.95,
        ..default()
    });
    let wall_mat = materials.add(StandardMaterial {
        base_color_texture: Some(load_repeating(
            asset_server,
            "textures/wall_bricks066_color.png",
        )),
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
    let arrow_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.9, 0.2),
        emissive: LinearRgba::rgb(0.6, 0.5, 0.05),
        ..default()
    });
    let door_fill_mats = [
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.65, 0.25),
            perceptual_roughness: 0.6,
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.70, 0.75),
            perceptual_roughness: 0.6,
            ..default()
        }),
    ];
    let door_lintel_mats = [
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.50, 0.38, 0.15),
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.18, 0.42, 0.45),
            ..default()
        }),
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
        floor_mat,
        wall_mat,
        ladder_mat,
        water_mat,
        fire_mat,
        poison_mat,
        horoscope_mat,
        arrow_mat,
        door_fill_mats,
        door_lintel_mats,
    }
}

/// A unit cube whose UVs are rebuilt so the texture reads upright on every side
/// face. Bevy's stock `Cuboid` mesh rotates the UVs 90° on the ±X faces, which
/// turns a brick texture into vertical stripes on two of the four walls.
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

/// Load a texture with repeat wrapping so tiling works (the default sampler
/// clamps to edge, which would smear the last pixel row).
fn load_repeating(asset_server: &AssetServer, path: &'static str) -> Handle<Image> {
    asset_server.load_with_settings(path, |settings: &mut ImageLoaderSettings| {
        settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            ..ImageSamplerDescriptor::default()
        });
    })
}
