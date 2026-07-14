//! Editor verification shot (`DEEPGRID_DEBUG_SHOT=editor`).
//!
//! bevy_egui draws directly to the window swap-chain, bypassing the render
//! target that Bevy's `Screenshot` copies — so an egui window is never in a
//! screenshot (mycity-simulator hit the same wall). Instead we render the editor
//! UI into an off-screen image via `EguiRenderToImage`, GPU-read that image back
//! to the CPU, and save the PNG ourselves.

use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy_egui::{EguiContext, EguiRenderToImage};

use super::ui::build_editor_ui;
use super::{EditorState, Tab};

/// The tab to open for the editor shot (set by `setup`).
#[derive(Resource)]
struct ShotTab(Tab);

/// Off-screen render size. Width is a multiple of 64 so the GPU readback has no
/// per-row padding (bytes-per-row is already 256-aligned) and the bytes map
/// straight to a tight RGBA8 image.
const SHOT_W: u32 = 1280;
const SHOT_H: u32 = 720;

/// Handle of the image the editor UI is rendered into, plus completion flag.
#[derive(Resource)]
pub struct EditorShot {
    image: Handle<Image>,
    saved: bool,
}

/// Register the render-to-image editor UI (opened on `tab`) and the capture driver.
pub fn setup(app: &mut App, tab: Tab) {
    app.insert_resource(ShotTab(tab))
        .add_systems(Startup, (spawn_render_target, open_tab))
        .add_systems(Update, (editor_ui_image, capture_driver));
}

/// Open the requested tab before the first frame is captured.
fn open_tab(shot_tab: Res<ShotTab>, mut state: ResMut<EditorState>) {
    state.tab = shot_tab.0;
    state.recompute_warnings();
}

fn spawn_render_target(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let size = Extent3d {
        width: SHOT_W,
        height: SHOT_H,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new_fill(
        size,
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::all(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    let handle = images.add(image);

    commands.spawn(EguiRenderToImage::new(handle.clone()));
    commands.insert_resource(EditorShot {
        image: handle,
        saved: false,
    });
}

/// Draw the editor into the render-to-image egui context each frame.
fn editor_ui_image(
    mut contexts: Query<&mut EguiContext, With<EguiRenderToImage>>,
    mut state: ResMut<EditorState>,
) {
    for mut ctx in &mut contexts {
        build_editor_ui(ctx.get_mut(), &mut state);
    }
}

/// After the UI has settled, request a readback of the image; when it completes,
/// save the PNG and exit.
fn capture_driver(
    mut frames: Local<u32>,
    mut requested: Local<bool>,
    mut commands: Commands,
    shot: Res<EditorShot>,
    mut exit: EventWriter<AppExit>,
) {
    *frames += 1;
    if *frames == 45 && !*requested {
        *requested = true;
        commands
            .spawn(Readback::texture(shot.image.clone()))
            .observe(on_readback);
    }
    if shot.saved {
        exit.send(AppExit::Success);
    }
}

fn on_readback(
    trigger: Trigger<ReadbackComplete>,
    mut commands: Commands,
    mut shot: ResMut<EditorShot>,
) {
    if shot.saved {
        return;
    }
    // Readback of a 1280-wide RGBA8 image is tightly packed (no row padding).
    let bytes = trigger.event().0.clone();
    let image = Image::new(
        Extent3d {
            width: SHOT_W,
            height: SHOT_H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::all(),
    );
    match image.try_into_dynamic() {
        Ok(dynamic) => {
            if let Err(e) = dynamic.save("debug-shot.png") {
                error!("editor shot: failed to save PNG: {e}");
            } else {
                info!("editor shot: saved debug-shot.png");
            }
        }
        Err(e) => error!("editor shot: bad image data: {e:?}"),
    }
    shot.saved = true;
    // Stop the per-frame readback.
    commands.entity(trigger.entity()).despawn();
}
