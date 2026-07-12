//! egui layer for the map editor. All data changes go through `EditorState`
//! methods (this file only reads state and forwards intents).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use super::{EditorState, PALETTE};
use crate::dungeon::{Block, Facing};

/// On-screen size of one grid cell, in points.
const CELL_PX: f32 = 16.0;

/// Human-readable palette label for a block.
fn block_name(block: Block) -> &'static str {
    match block {
        Block::Wall => "Wall",
        Block::Empty => "Empty",
        Block::Water => "Water",
        Block::Fire => "Fire",
        Block::Poison => "Poison",
        Block::Ladder => "Ladder",
        Block::Door { kind: 0 } => "Door 1",
        Block::Door { .. } => "Door 2",
        Block::Horoscope { pass_from: Facing::West } => "Horo <W",
        Block::Horoscope { pass_from: Facing::East } => "Horo >E",
        Block::Horoscope { pass_from: Facing::North } => "Horo ^N",
        Block::Horoscope { pass_from: Facing::South } => "Horo vS",
    }
}

/// Fill color for a cell. Empty cells split floor (has footing) vs void (a hole),
/// so support/holes read at a glance.
fn cell_color(block: Block, footing: bool) -> egui::Color32 {
    match block {
        Block::Wall => egui::Color32::from_rgb(70, 70, 78),
        Block::Empty if footing => egui::Color32::from_rgb(180, 180, 186),
        Block::Empty => egui::Color32::from_rgb(28, 28, 36),
        Block::Water => egui::Color32::from_rgb(40, 90, 190),
        Block::Fire => egui::Color32::from_rgb(200, 90, 30),
        Block::Poison => egui::Color32::from_rgb(70, 170, 60),
        Block::Ladder => egui::Color32::from_rgb(140, 95, 45),
        Block::Door { kind: 0 } => egui::Color32::from_rgb(200, 150, 60),
        Block::Door { .. } => egui::Color32::from_rgb(70, 170, 180),
        Block::Horoscope { .. } => egui::Color32::from_rgb(150, 70, 200),
    }
}

/// Glyph drawn on cells whose color alone doesn't convey the type.
fn cell_glyph(block: Block) -> Option<char> {
    match block {
        Block::Ladder => Some('H'),
        Block::Door { kind: 0 } => Some('1'),
        Block::Door { .. } => Some('2'),
        Block::Horoscope { pass_from } => Some(match pass_from {
            Facing::West => '<',
            Facing::East => '>',
            Facing::North => '^',
            Facing::South => 'v',
        }),
        _ => None,
    }
}

/// Which grid cell (if any) a screen point falls on.
fn cell_of(rect: egui::Rect, p: egui::Pos2, w: usize, h: usize) -> Option<(i32, i32)> {
    if !rect.contains(p) {
        return None;
    }
    let x = ((p.x - rect.min.x) / CELL_PX) as i32;
    let y = ((p.y - rect.min.y) / CELL_PX) as i32;
    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
        Some((x, y))
    } else {
        None
    }
}

/// Editor UI driven by the primary window's egui context (interactive mode).
pub fn editor_ui_window(mut contexts: EguiContexts, mut state: ResMut<EditorState>) {
    let ctx = contexts.ctx_mut();
    build_editor_ui(ctx, &mut state);
}

/// Build the whole editor UI into `ctx`. Reused for the window context
/// (interactive) and for the render-to-image context (the `editor` debug shot),
/// since egui overlays on the window aren't captured by Bevy screenshots.
pub fn build_editor_ui(ctx: &mut egui::Context, state: &mut EditorState) {
    // Keyboard shortcuts (read first, act after — the closure can't borrow state).
    let (undo, redo, save) = ctx.input(|i| {
        let cmd = i.modifiers.ctrl || i.modifiers.command;
        (
            cmd && i.key_pressed(egui::Key::Z) && !i.modifiers.shift,
            cmd && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
            cmd && i.key_pressed(egui::Key::S),
        )
    });
    if undo {
        state.undo();
    }
    if redo {
        state.redo();
    }
    if save {
        state.save();
    }

    top_bar(ctx, state);
    palette_panel(ctx, state);
    status_bar(ctx, state);
    grid_panel(ctx, state);
}

fn top_bar(ctx: &egui::Context, state: &mut EditorState) {
    egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            let star = if state.is_dirty() { " *" } else { "" };
            ui.strong(format!("{}{star}", state.project_name));
            ui.separator();

            let n_levels = state.levels.len();
            let mut level = state.level_index;
            egui::ComboBox::from_label("Level")
                .selected_text(format!("{level}"))
                .show_ui(ui, |ui| {
                    for i in 0..n_levels {
                        ui.selectable_value(&mut level, i, format!("{i}"));
                    }
                });
            state.select_level(level);

            let n_floors = state.cur().floor_count();
            let mut floor = state.floor_index;
            egui::ComboBox::from_label("Floor")
                .selected_text(format!("{floor}"))
                .show_ui(ui, |ui| {
                    for i in 0..n_floors {
                        ui.selectable_value(&mut floor, i, format!("{i}"));
                    }
                });
            state.select_floor(floor);

            ui.separator();
            if ui.button("Save").clicked() {
                state.save();
            }
            if ui.add_enabled(state.can_undo(), egui::Button::new("Undo")).clicked() {
                state.undo();
            }
            if ui.add_enabled(state.can_redo(), egui::Button::new("Redo")).clicked() {
                state.redo();
            }
        });
    });
}

fn palette_panel(ctx: &egui::Context, state: &mut EditorState) {
    egui::SidePanel::left("palette")
        .resizable(false)
        .default_width(110.0)
        .show(ctx, |ui| {
            ui.heading("Blocks");
            for &block in PALETTE {
                let selected = state.selected == block;
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 22.0),
                    egui::Sense::click(),
                );
                if selected {
                    ui.painter()
                        .rect_filled(rect, 3.0, egui::Color32::from_gray(90));
                }
                let swatch = egui::Rect::from_min_size(
                    rect.min + egui::vec2(4.0, 3.0),
                    egui::vec2(16.0, 16.0),
                );
                ui.painter().rect_filled(swatch, 2.0, cell_color(block, true));
                ui.painter().text(
                    rect.min + egui::vec2(28.0, 11.0),
                    egui::Align2::LEFT_CENTER,
                    block_name(block),
                    egui::FontId::proportional(13.0),
                    ui.visuals().text_color(),
                );
                if response.clicked() {
                    state.selected = block;
                }
            }
        });
}

fn status_bar(ctx: &egui::Context, state: &EditorState) {
    egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            match state.cursor {
                Some((x, y)) => ui.label(format!("({x}, {y}, f{})", state.floor_index)),
                None => ui.label("(-, -)"),
            };
            ui.separator();
            ui.label(format!("Block: {}", block_name(state.selected)));
            ui.separator();
            let s = state.cur().start;
            ui.label(format!(
                "Start: ({}, {}, f{}) {:?}",
                s.x, s.y, s.floor, state.cur().start_facing
            ));
            ui.separator();
            if state.is_dirty() {
                ui.colored_label(egui::Color32::from_rgb(240, 210, 60), "UNSAVED");
                ui.separator();
            }
            ui.label(&state.status);
        });
    });
}

fn grid_panel(ctx: &egui::Context, state: &mut EditorState) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let (w, h) = (state.cur().width(), state.cur().height());
        egui::ScrollArea::both().show(ui, |ui| {
            let size = egui::vec2(w as f32 * CELL_PX, h as f32 * CELL_PX);
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
            let painter = ui.painter_at(rect);

            for y in 0..h {
                for x in 0..w {
                    let cell = egui::Rect::from_min_size(
                        egui::pos2(rect.min.x + x as f32 * CELL_PX, rect.min.y + y as f32 * CELL_PX),
                        egui::vec2(CELL_PX, CELL_PX),
                    );
                    let block = state.block_at(x as i32, y as i32).unwrap_or(Block::Empty);
                    let footing = state.has_footing(x as i32, y as i32);
                    painter.rect_filled(cell, 0.0, cell_color(block, footing));

                    // Faint underlay: a wall on the floor below (support visible).
                    if state.wall_below(x as i32, y as i32) && !block.is_wall() {
                        painter.rect_stroke(
                            cell.shrink(2.0),
                            0.0,
                            egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 120, 170)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if let Some(g) = cell_glyph(block) {
                        painter.text(
                            cell.center(),
                            egui::Align2::CENTER_CENTER,
                            g,
                            egui::FontId::monospace(CELL_PX * 0.85),
                            egui::Color32::BLACK,
                        );
                    }
                }
            }

            // Grid lines.
            let line = egui::Stroke::new(1.0_f32, egui::Color32::from_black_alpha(60));
            for x in 0..=w {
                let px = rect.min.x + x as f32 * CELL_PX;
                painter.line_segment([egui::pos2(px, rect.min.y), egui::pos2(px, rect.max.y)], line);
            }
            for y in 0..=h {
                let py = rect.min.y + y as f32 * CELL_PX;
                painter.line_segment([egui::pos2(rect.min.x, py), egui::pos2(rect.max.x, py)], line);
            }

            // Start marker (only when the start is on the shown floor).
            let start = state.cur().start;
            if start.floor == state.floor_index {
                let center = egui::pos2(
                    rect.min.x + (start.x as f32 + 0.5) * CELL_PX,
                    rect.min.y + (start.y as f32 + 0.5) * CELL_PX,
                );
                let gold = egui::Color32::from_rgb(255, 220, 40);
                painter.circle_filled(center, CELL_PX * 0.28, gold);
                let (dx, dy) = state.cur().start_facing.delta();
                let tip = center + egui::vec2(dx as f32, dy as f32) * (CELL_PX * 0.45);
                painter.line_segment([center, tip], egui::Stroke::new(2.5_f32, gold));
            }

            // Interaction: left paint (drag stroke), right-click sets the start.
            let hover = response.hover_pos().and_then(|p| cell_of(rect, p, w, h));
            state.cursor = hover;
            if (response.dragged() || response.clicked())
                && let Some((cx, cy)) =
                    response.interact_pointer_pos().and_then(|p| cell_of(rect, p, w, h))
            {
                state.paint(cx, cy);
            }
            if response.secondary_clicked()
                && let Some((cx, cy)) = hover
            {
                state.set_start(cx, cy);
            }
            if response.drag_stopped() || response.clicked() {
                state.end_stroke();
            }
        });
    });
}
