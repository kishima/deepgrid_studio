use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::prelude::*;

use crate::dungeon::{Block, Dungeon};

/// World size of one grid block (1.0 × 1.0 × 1.0). A tile `(x, y)` occupies the
/// world box `[x, x+1] × [0, 1] × [y, y+1]`, so its center is `(x+0.5, _, y+0.5)`.
pub const BLOCK_SIZE: f32 = 1.0;

/// Build the static dungeon geometry for the current floor: one floor slab, one
/// ceiling slab, and a cube per wall block.
///
/// plan1 keeps this deliberately naive (project.md「実装を単純にするため」): a
/// separate `Cuboid` per wall, single-color materials, no face culling. At the
/// default 40×40 that is a few hundred cubes — fine for the software renderer.
pub fn setup_dungeon(
    mut commands: Commands,
    dungeon: Res<Dungeon>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let floor = dungeon.current_floor();
    let (w, h) = (floor.width as f32, floor.height as f32);

    // CC0 textures from ambientCG (provenance: CREDITS.md). Color maps
    // only for now — normal/roughness maps would need tangents and cost more
    // on the software renderer.
    let floor_material = materials.add(StandardMaterial {
        base_color_texture: Some(load_repeating(
            &asset_server,
            "textures/floor_pavingstones119_color.png",
        )),
        perceptual_roughness: 0.95,
        // The floor slab is one quad over the whole map; tile one texture per block.
        uv_transform: Affine2::from_scale(Vec2::new(w, h)),
        ..default()
    });
    let ceiling_material = materials.add(StandardMaterial {
        base_color_texture: Some(load_repeating(
            &asset_server,
            "textures/ceiling_rock058_color.png",
        )),
        perceptual_roughness: 0.95,
        uv_transform: Affine2::from_scale(Vec2::new(w, h)),
        ..default()
    });
    // Wall cubes map the full texture onto each face (one block = one tile),
    // so no uv_transform is needed.
    let wall_material = materials.add(StandardMaterial {
        base_color_texture: Some(load_repeating(
            &asset_server,
            "textures/wall_bricks066_color.png",
        )),
        perceptual_roughness: 0.9,
        ..default()
    });

    // Floor slab: a thin box whose top face sits exactly at y = 0.
    let slab = meshes.add(Cuboid::new(w * BLOCK_SIZE, 0.02, h * BLOCK_SIZE));
    commands.spawn((
        Mesh3d(slab.clone()),
        MeshMaterial3d(floor_material),
        Transform::from_xyz(w * BLOCK_SIZE / 2.0, -0.01, h * BLOCK_SIZE / 2.0),
    ));
    // Ceiling slab: same box, bottom face at y = 1.0.
    commands.spawn((
        Mesh3d(slab),
        MeshMaterial3d(ceiling_material),
        Transform::from_xyz(
            w * BLOCK_SIZE / 2.0,
            BLOCK_SIZE + 0.01,
            h * BLOCK_SIZE / 2.0,
        ),
    ));

    // One cube per wall block, centered in its tile.
    let cube = meshes.add(wall_cube_mesh());
    for y in 0..floor.height {
        for x in 0..floor.width {
            if floor.get(x as i32, y as i32) == Some(Block::Wall) {
                commands.spawn((
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(wall_material.clone()),
                    Transform::from_xyz(
                        x as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
                        BLOCK_SIZE / 2.0,
                        y as f32 * BLOCK_SIZE + BLOCK_SIZE / 2.0,
                    ),
                ));
            }
        }
    }

    // Ambient fill so wall faces turned away from the player's light stay
    // readable instead of going black.
    // High ambient base so the whole floor stays readable and the player light
    // is a soft accent rather than a harsh hotspot (uniform, Grimrock-ish look).
    commands.insert_resource(AmbientLight {
        color: Color::srgb(0.7, 0.75, 0.9),
        brightness: 700.0,
    });
}

/// A unit cube whose UVs are rebuilt so the texture reads upright on every
/// side face. Bevy's stock `Cuboid` mesh rotates the UVs 90° on the ±X faces,
/// which turns a brick texture into vertical stripes on two of the four walls.
/// UVs are derived from vertex position by the face normal's dominant axis,
/// so this stays correct regardless of the builder's vertex ordering.
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
        .map(|(p, n)| {
            if n[0].abs() > 0.5 {
                // ±X side: u along Z, v down from the top edge.
                [p[2] + 0.5, 0.5 - p[1]]
            } else if n[2].abs() > 0.5 {
                // ±Z side: u along X, v down from the top edge.
                [p[0] + 0.5, 0.5 - p[1]]
            } else {
                // Top/bottom: plain planar mapping.
                [p[0] + 0.5, p[2] + 0.5]
            }
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh
}

/// Load a texture with repeat wrapping so `uv_transform` tiling works (the
/// default sampler clamps to edge, which would smear the last pixel row).
fn load_repeating(asset_server: &AssetServer, path: &'static str) -> Handle<Image> {
    asset_server.load_with_settings(path, |settings: &mut ImageLoaderSettings| {
        settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            ..ImageSamplerDescriptor::default()
        });
    })
}
