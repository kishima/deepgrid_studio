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
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let floor = dungeon.current_floor();
    let (w, h) = (floor.width as f32, floor.height as f32);

    let floor_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.32, 0.29, 0.26),
        perceptual_roughness: 0.95,
        ..default()
    });
    let ceiling_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.24, 0.25, 0.30),
        perceptual_roughness: 0.95,
        ..default()
    });
    let wall_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.53, 0.60),
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
    let cube = meshes.add(Cuboid::new(BLOCK_SIZE, BLOCK_SIZE, BLOCK_SIZE));
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
    commands.insert_resource(AmbientLight {
        color: Color::srgb(0.7, 0.75, 0.9),
        brightness: 400.0,
    });
}
