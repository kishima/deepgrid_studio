//! 3D party portraits (plan4). Each party member's glTF is rendered as a
//! bust-up into its own 128×128 image, which the HUD status card shows via an
//! `ImageNode`. This stands in for the original's hand-drawn "顔グラフィック";
//! user-supplied images arrive with the graphics-swap system in plan10, so no
//! PNG portraits are baked here.
//!
//! Isolation without RenderLayers: Bevy 0.15 does *not* propagate a `RenderLayers`
//! component from a `SceneRoot` down to the meshes the glTF spawns, so tagging the
//! root wouldn't keep the model out of the main view. Instead each portrait rig
//! (model + light + camera) is parked far *below* the dungeon and the portrait
//! camera is given a short far-plane, so its frustum contains only its own model
//! and the main camera (fixed-pitch, horizontal) never looks down far enough to
//! see the rigs.

use bevy::pbr::ClusterConfig;
use bevy::prelude::*;
use bevy::render::camera::RenderTarget;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureUsages,
};

use crate::character::Party;
use crate::props::animated_scene;

/// Portrait render-target resolution (square).
const PORTRAIT_SIZE: u32 = 128;

/// `Idle` clip index in the KayKit Adventurers glbs (identical rig across the
/// pack — 76 clips; verified against knight.glb).
const ANIM_IDLE_ADVENTURER: usize = 36;

/// World Y the portrait rigs sit at — far below the dungeon (floors span y≈0..5)
/// so the fixed-pitch main camera never frames them.
const RIG_Y: f32 = -1000.0;

/// Horizontal spacing between rigs so no portrait camera's (short-far-plane)
/// frustum catches a neighbour.
const RIG_SPACING: f32 = 20.0;

/// Camera height above the model's feet (frames roughly head→chest).
const BUST_TARGET_Y: f32 = 1.45;
/// Camera distance in front of the model (+Z is the KayKit facing).
const BUST_DISTANCE: f32 = 1.2;
/// Vertical FOV framing the bust.
const BUST_FOV_DEG: f32 = 38.0;

/// Frames the portrait camera renders before freezing (interactive play only):
/// after this it goes inactive and the last-rendered bust persists as a still,
/// per plan4's lavapipe fallback. Debug-shot runs never freeze (they exit fast),
/// so captures always show a live portrait.
const PORTRAIT_LIVE_FRAMES: u32 = 120;

/// Handles to each party slot's portrait image, indexed by slot. The HUD reads
/// this to fill its `ImageNode`s.
#[derive(Resource, Default)]
pub struct Portraits {
    pub images: Vec<Handle<Image>>,
}

/// Marks a portrait camera so the freeze fallback can find them.
#[derive(Component)]
pub struct PortraitCamera;

/// Build a 128×128 image usable as a camera render target.
fn make_target(images: &mut Assets<Image>) -> Handle<Image> {
    let size = Extent3d {
        width: PORTRAIT_SIZE,
        height: PORTRAIT_SIZE,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new_fill(
        size,
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING
        | TextureUsages::COPY_DST
        | TextureUsages::RENDER_ATTACHMENT;
    images.add(image)
}

/// Spawn one render-to-texture portrait rig per party member: the model (looping
/// Idle), a fill light, and a camera targeting a fresh image. Populates the
/// [`Portraits`] resource. No-op for an empty party.
pub fn setup_portraits(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    party: Res<Party>,
    mut images: ResMut<Assets<Image>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    let mut handles = Vec::with_capacity(party.len());

    for (i, member) in party.members.iter().enumerate() {
        let target = make_target(&mut images);
        handles.push(target.clone());

        let base = Vec3::new(i as f32 * RIG_SPACING, RIG_Y, 0.0);

        // Model, feet at `base`, facing +Z (toward the camera), Idle looping.
        let (scene, anim) =
            animated_scene(&asset_server, &mut graphs, &member.character.model, ANIM_IDLE_ADVENTURER);
        commands.spawn((scene, anim, Transform::from_translation(base)));

        // Fill light close in front of the model (range-limited so it can't reach
        // the dungeon far above).
        commands.spawn((
            PointLight {
                intensity: 90_000.0,
                range: 6.0,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_translation(base + Vec3::new(0.6, 1.7, 1.4)),
        ));

        // Camera looking at the bust. Short far-plane so only this model (nothing
        // in the dungeon far above) is inside the frustum. Rendered before the
        // main view (negative order) into the target image.
        let eye = base + Vec3::new(0.0, BUST_TARGET_Y, BUST_DISTANCE);
        let look = base + Vec3::new(0.0, BUST_TARGET_Y - 0.05, 0.0);
        commands.spawn((
            PortraitCamera,
            Camera3d::default(),
            Camera {
                target: RenderTarget::Image(target),
                order: -10 - i as isize,
                clear_color: ClearColorConfig::Custom(Color::srgb(0.06, 0.07, 0.10)),
                ..default()
            },
            Msaa::Off,
            // A lone point light per rig would otherwise paint a bright rectangle
            // on the software renderer (same issue as the player light, plan1).
            ClusterConfig::Single,
            Projection::Perspective(PerspectiveProjection {
                fov: BUST_FOV_DEG.to_radians(),
                near: 0.1,
                far: 10.0,
                ..default()
            }),
            Transform::from_translation(eye).looking_at(look, Vec3::Y),
        ));
    }

    commands.insert_resource(Portraits { images: handles });
}

/// Interactive-play fallback: once the portraits have rendered for a short while,
/// deactivate their cameras so four extra 3D views don't tax the software
/// renderer. Animation then stops (acceptable per plan4). Skipped during
/// debug-shot runs so captures render a live portrait.
pub fn freeze_portraits(
    mut frames: Local<u32>,
    mut done: Local<bool>,
    mut cameras: Query<&mut Camera, With<PortraitCamera>>,
) {
    if *done {
        return;
    }
    if crate::debug_shot::debug_shot_enabled() {
        return;
    }
    *frames += 1;
    if *frames >= PORTRAIT_LIVE_FRAMES {
        for mut cam in &mut cameras {
            cam.is_active = false;
        }
        *done = true;
    }
}
