//! egui layer for the Studio editor (plan9). Reads [`EditorState`] and calls its
//! methods — the UI never mutates project data except through those methods, so
//! dirty tracking / snapshots stay centralised. One tab bar switches between the
//! map editor and the five content editors + project settings.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use super::labels::{
    block_label, flag_join_label, growth_label, move_mode_label, move_type_label, plate_cond_label,
};
use super::{EditorState, PALETTE, PlaceLayer, Tab};
use crate::character::{GrowthType, StatKind};
use crate::dungeon::{Block, Facing};
use crate::event::{EventAction, FlagJoin, LiquidKind, MoveMode, PlateCond, TriggerKind};
use crate::item::{EquipSlot, ItemKind};
use crate::magic::MagicKind;
use crate::monster::MoveType;

const CELL_PX: f32 = 16.0;

/// Editor UI driven by the primary window's egui context (interactive mode).
pub fn editor_ui_window(mut contexts: EguiContexts, mut state: ResMut<EditorState>) {
    build_editor_ui(contexts.ctx_mut(), &mut state);
}

/// Install the bundled Japanese pixel font so egui labels render (egui's default
/// font has no CJK glyphs), and bump the default text sizes (2026-07 feedback:
/// エディットの画面の文字が小さい). Done once per context.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "jp".to_owned(),
        egui::FontData::from_static(include_bytes!("../../assets/fonts/PixelMplus12-Regular.ttf")).into(),
    );
    fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "jp".to_owned());
    fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "jp".to_owned());
    ctx.set_fonts(fonts);
    ctx.style_mut(|style| {
        use egui::{FontFamily, FontId, TextStyle};
        style.text_styles.insert(TextStyle::Small, FontId::new(12.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Body, FontId::new(16.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Button, FontId::new(16.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Monospace, FontId::new(14.0, FontFamily::Monospace));
        style.text_styles.insert(TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional));
    });
}

/// Build the whole editor UI. Reused for the window and the render-to-image shot.
pub fn build_editor_ui(ctx: &mut egui::Context, state: &mut EditorState) {
    if !state.fonts_installed {
        install_fonts(ctx);
        state.fonts_installed = true;
    }
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
    bottom_bar(ctx, state);
    match state.tab {
        Tab::Map => map_view(ctx, state),
        Tab::Characters => characters_tab(ctx, state),
        Tab::Items => items_tab(ctx, state),
        Tab::Monsters => monsters_tab(ctx, state),
        Tab::Magics => magics_tab(ctx, state),
        Tab::Events => events_tab(ctx, state),
        Tab::Settings => settings_tab(ctx, state),
    }
}

// ------------------------------------------------------------------ frame

fn top_bar(ctx: &egui::Context, state: &mut EditorState) {
    egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            let star = if state.is_dirty() { " *" } else { "" };
            ui.strong(format!("{}{star}", state.proj.name));
            ui.separator();
            for tab in Tab::ALL {
                if ui.selectable_label(state.tab == tab, tab.label()).clicked() && state.tab != tab {
                    state.tab = tab;
                    state.recompute_warnings();
                }
            }
            ui.separator();
            if ui.button("Save All").clicked() {
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

fn bottom_bar(ctx: &egui::Context, state: &EditorState) {
    egui::TopBottomPanel::bottom("bottom_bar").show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            let l = &state.proj.limits;
            let color = if state.warnings.is_empty() {
                egui::Color32::from_rgb(120, 200, 120)
            } else {
                egui::Color32::from_rgb(240, 200, 80)
            };
            ui.colored_label(color, format!("警告 {}件", state.warnings.len()));
            ui.separator();
            ui.label(format!(
                "アイテム {}/{}  モンスター {}/{}  魔法 {}/{}  キャラ {}/{}  レベル {}/{}",
                state.proj.items.len(), l.max_item_kinds,
                state.proj.monsters.len(), l.max_monster_kinds,
                state.proj.magics.len(), l.max_magic_kinds,
                state.proj.characters.len(), l.max_characters,
                state.proj.levels.len(), l.max_levels,
            ));
            ui.separator();
            ui.label(&state.status);
        });
        if !state.warnings.is_empty() {
            egui::CollapsingHeader::new("警告一覧").show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(90.0).show(ui, |ui| {
                    for w in &state.warnings {
                        ui.colored_label(egui::Color32::from_rgb(240, 200, 80), w);
                    }
                });
            });
        }
    });
}

// ------------------------------------------------------------------ helpers

/// A left-hand list panel with +新規 / 複製 / 削除 buttons. Returns the chosen
/// action so the caller (which owns the concrete vec) can apply it.
enum ListAction {
    None,
    Add,
    Duplicate,
    Delete,
}

fn list_panel(
    ui: &mut egui::Ui,
    title: &str,
    labels: &[String],
    sel: &mut usize,
) -> ListAction {
    let mut action = ListAction::None;
    ui.heading(title);
    ui.horizontal(|ui| {
        if ui.button("＋新規").clicked() {
            action = ListAction::Add;
        }
        if ui.add_enabled(!labels.is_empty(), egui::Button::new("複製")).clicked() {
            action = ListAction::Duplicate;
        }
        if ui.add_enabled(!labels.is_empty(), egui::Button::new("削除")).clicked() {
            action = ListAction::Delete;
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, label) in labels.iter().enumerate() {
            if ui.selectable_label(*sel == i, label).clicked() {
                *sel = i;
            }
        }
    });
    action
}

/// A dropdown to pick an id from `ids` (with a blank option). Returns true if changed.
fn id_combo(ui: &mut egui::Ui, id: &str, current: &mut String, ids: &[String]) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(if current.is_empty() { "―" } else { current.as_str() })
        .show_ui(ui, |ui| {
            if ui.selectable_value(current, String::new(), "―").changed() {
                changed = true;
            }
            for opt in ids {
                if ui.selectable_value(current, opt.clone(), opt).changed() {
                    changed = true;
                }
            }
        });
    changed
}

/// Edit a `Vec<String>` of ids (add / remove rows).
fn id_list(ui: &mut egui::Ui, salt: &str, list: &mut Vec<String>, ids: &[String]) -> bool {
    let mut changed = false;
    let mut remove = None;
    for (i, entry) in list.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            if id_combo(ui, &format!("{salt}_{i}"), entry, ids) {
                changed = true;
            }
            if ui.small_button("×").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = true;
    }
    if ui.button("＋行").clicked() {
        list.push(ids.first().cloned().unwrap_or_default());
        changed = true;
    }
    changed
}

fn ids_of<T, F: Fn(&T) -> String>(v: &[T], f: F) -> Vec<String> {
    v.iter().map(f).collect()
}

// ------------------------------------------------------------------ map tab

fn map_view(ctx: &egui::Context, state: &mut EditorState) {
    egui::SidePanel::left("palette").resizable(false).default_width(150.0).show(ctx, |ui| {
        ui.heading("レイヤー");
        for (layer, name) in [
            (PlaceLayer::Block, "ブロック"),
            (PlaceLayer::Item, "アイテム"),
            (PlaceLayer::Monster, "モンスター"),
            (PlaceLayer::Trigger, "トリガー"),
        ] {
            ui.selectable_value(&mut state.place_layer, layer, name);
        }
        ui.separator();
        match state.place_layer {
            PlaceLayer::Block => {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for &block in PALETTE {
                        if ui.selectable_label(state.selected == block, block_label(block)).clicked() {
                            state.selected = block;
                        }
                    }
                });
            }
            PlaceLayer::Item => {
                let ids = ids_of(&state.proj.items, |d| d.id.clone());
                id_combo(ui, "place_item", &mut state.place_item, &ids);
            }
            PlaceLayer::Monster => {
                let ids = ids_of(&state.proj.monsters, |d| d.id.clone());
                id_combo(ui, "place_monster", &mut state.place_monster, &ids);
            }
            PlaceLayer::Trigger => {
                for b in [Block::Keyhole, Block::Switch, Block::FloorPlate, Block::WarpPoint] {
                    ui.selectable_value(&mut state.place_trigger, b, block_label(b));
                }
                ui.small("置くとイベント雛形を自動生成");
            }
        }
    });

    egui::TopBottomPanel::top("map_top").show(ctx, |ui| {
        ui.horizontal(|ui| {
            let n_levels = state.proj.levels.len();
            let mut level = state.level_index;
            egui::ComboBox::from_label("レベル").selected_text(format!("{level}")).show_ui(ui, |ui| {
                for i in 0..n_levels {
                    ui.selectable_value(&mut level, i, format!("{i}"));
                }
            });
            state.select_level(level);
            let n_floors = state.cur().floor_count();
            let mut floor = state.floor_index;
            egui::ComboBox::from_label("フロア").selected_text(format!("{floor}")).show_ui(ui, |ui| {
                for i in 0..n_floors {
                    ui.selectable_value(&mut floor, i, format!("{i}"));
                }
            });
            state.select_floor(floor);
            ui.separator();
            if ui.button("＋レベル").clicked() {
                state.add_level();
            }
            if ui.add_enabled(n_levels > 1, egui::Button::new("－レベル")).clicked() {
                let idx = state.level_index;
                state.delete_level(idx);
            }
            ui.separator();
            // 2D / 3D toggle (plan9.5).
            let mut mode_3d = state.mode_3d;
            ui.selectable_value(&mut mode_3d, false, "2D");
            ui.selectable_value(&mut mode_3d, true, "3D");
            state.mode_3d = mode_3d;
            if state.mode_3d {
                ui.separator();
                ui.label(format!("視点 {}", state.d3_coord));
                ui.label("WASD/QE歩行 R/F昇降 左設置/右消去");
            } else {
                ui.separator();
                let lvl = state.cur();
                ui.label(format!("配置: アイテム {} / モンスター {}", lvl.items.len(), lvl.monsters.len()));
            }
        });
    });

    // In 3D mode the central area shows the 3D walk view (drawn by Bevy under
    // egui); skip the 2D grid so the transparent egui centre reveals it.
    if !state.mode_3d {
        egui::CentralPanel::default().show(ctx, |ui| {
            grid(ui, state);
        });
    }
}

fn grid(ui: &mut egui::Ui, state: &mut EditorState) {
    let (w, h) = (state.cur().width(), state.cur().height());
    let floor = state.floor_index;
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
                painter.rect_filled(cell, 0.0, cell_color(block, state.has_footing(x as i32, y as i32)));
                if state.wall_below(x as i32, y as i32) && !block.is_solid() {
                    painter.rect_stroke(cell.shrink(2.0), 0.0, egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(90, 120, 170)), egui::StrokeKind::Inside);
                }
                if let Some(g) = cell_glyph(block) {
                    painter.text(cell.center(), egui::Align2::CENTER_CENTER, g, egui::FontId::monospace(CELL_PX * 0.85), egui::Color32::BLACK);
                }
            }
        }
        // Placement markers on this floor.
        for p in state.cur().items.iter().filter(|p| p.floor == floor) {
            marker(&painter, rect, p.x, p.y, egui::Color32::from_rgb(240, 230, 90), "i");
        }
        for p in state.cur().monsters.iter().filter(|p| p.floor == floor) {
            marker(&painter, rect, p.x, p.y, egui::Color32::from_rgb(240, 120, 120), "m");
        }
        let line = egui::Stroke::new(1.0_f32, egui::Color32::from_black_alpha(60));
        for x in 0..=w {
            let px = rect.min.x + x as f32 * CELL_PX;
            painter.line_segment([egui::pos2(px, rect.min.y), egui::pos2(px, rect.max.y)], line);
        }
        for y in 0..=h {
            let py = rect.min.y + y as f32 * CELL_PX;
            painter.line_segment([egui::pos2(rect.min.x, py), egui::pos2(rect.max.x, py)], line);
        }
        let start = state.cur().start;
        if start.floor == floor {
            let center = egui::pos2(rect.min.x + (start.x as f32 + 0.5) * CELL_PX, rect.min.y + (start.y as f32 + 0.5) * CELL_PX);
            let gold = egui::Color32::from_rgb(255, 220, 40);
            painter.circle_filled(center, CELL_PX * 0.28, gold);
            let (dx, dy) = state.cur().start_facing.delta();
            painter.line_segment([center, center + egui::vec2(dx as f32, dy as f32) * (CELL_PX * 0.45)], egui::Stroke::new(2.5_f32, gold));
        }
        let hover = response.hover_pos().and_then(|p| cell_of(rect, p, w, h));
        state.cursor = hover;
        if (response.dragged() || response.clicked())
            && let Some((cx, cy)) = response.interact_pointer_pos().and_then(|p| cell_of(rect, p, w, h))
        {
            if state.place_layer == PlaceLayer::Block {
                state.paint(cx, cy);
            } else if response.clicked() {
                state.place_at(cx, cy);
            }
        }
        if response.secondary_clicked()
            && let Some((cx, cy)) = hover
        {
            if state.place_layer == PlaceLayer::Block {
                state.set_start(cx, cy);
            } else {
                state.erase_at(cx, cy);
            }
        }
        if response.drag_stopped() || response.clicked() {
            state.end_stroke();
        }
    });
}

fn marker(painter: &egui::Painter, rect: egui::Rect, x: i32, y: i32, color: egui::Color32, glyph: &str) {
    let c = egui::pos2(rect.min.x + (x as f32 + 0.5) * CELL_PX, rect.min.y + (y as f32 + 0.5) * CELL_PX);
    painter.circle_filled(c, CELL_PX * 0.32, color);
    painter.text(c, egui::Align2::CENTER_CENTER, glyph, egui::FontId::monospace(CELL_PX * 0.7), egui::Color32::BLACK);
}

// ------------------------------------------------------------------ characters

fn characters_tab(ctx: &egui::Context, state: &mut EditorState) {
    let labels = ids_of(&state.proj.characters, |c| format!("{} {}", c.id, c.first_name));
    let mut sel = state.sel_char;
    egui::SidePanel::left("char_list").default_width(180.0).show(ctx, |ui| {
        match list_panel(ui, "キャラ一覧", &labels, &mut sel) {
            ListAction::Add => {
                state.snapshot();
                let ids = ids_of(&state.proj.characters, |c| c.id.clone());
                state.proj.characters.push(new_character(super::ops::next_id(&ids, super::ops::IdKind::Character.prefix())));
                sel = state.proj.characters.len() - 1;
            }
            ListAction::Duplicate => {
                state.snapshot();
                let mut c = state.proj.characters[sel].clone();
                let ids = ids_of(&state.proj.characters, |c| c.id.clone());
                c.id = super::ops::next_id(&ids, super::ops::IdKind::Character.prefix());
                state.proj.characters.push(c);
                sel = state.proj.characters.len() - 1;
            }
            ListAction::Delete => {
                state.snapshot();
                state.proj.characters.remove(sel);
                sel = sel.min(state.proj.characters.len().saturating_sub(1));
            }
            ListAction::None => {}
        }
    });
    state.sel_char = sel;

    egui::CentralPanel::default().show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            party_editor(ui, state);
            ui.separator();
            let Some(c) = state.proj.characters.get(state.sel_char).cloned() else {
                ui.label("キャラを選択");
                return;
            };
            let item_ids = ids_of(&state.proj.items, |d| d.id.clone());
            let magic_ids = ids_of(&state.proj.magics, |d| d.id.clone());
            rename_row(ui, state, super::ops::IdKind::Character, c.id.clone());
            let ch = &mut state.proj.characters[state.sel_char];
            let mut dirty = false;
            dirty |= text_row(ui, "名", &mut ch.first_name);
            dirty |= text_row(ui, "姓", &mut ch.last_name);
            dirty |= text_row(ui, "性別", &mut ch.gender);
            ui.horizontal(|ui| {
                ui.label("成長型");
                for g in [GrowthType::Average, GrowthType::EarlyBloomer, GrowthType::LateBloomer, GrowthType::Genius, GrowthType::Talentless] {
                    if ui.selectable_value(&mut ch.growth, g, growth_label(g)).changed() {
                        dirty = true;
                    }
                }
            });
            ui.horizontal(|ui| {
                dirty |= num_row(ui, "身長", &mut ch.height_cm);
                dirty |= num_row(ui, "体重", &mut ch.weight_kg);
                dirty |= num_u(ui, "年齢", &mut ch.age);
            });
            dirty |= text_row(ui, "経歴", &mut ch.background);
            ui.collapsing("能力値", |ui| {
                dirty |= num_u(ui, "レベル", &mut ch.stats.level);
                let s = &mut ch.stats;
                for (label, field) in stat_fields(s) {
                    dirty |= num_i(ui, label, field);
                }
            });
            dirty |= text_row(ui, "モデル", &mut ch.model);
            dirty |= text_row(ui, "ポートレート", &mut ch.portrait);
            ui.label("初期アイテム");
            dirty |= id_list(ui, "char_items", &mut ch.items, &item_ids);
            ui.label("初期習得魔法");
            dirty |= id_list(ui, "char_magics", &mut ch.magics, &magic_ids);
            if dirty {
                state.touch();
            }
        });
    });
}

fn party_editor(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.collapsing("パーティ編成", |ui| {
        let char_ids = ids_of(&state.proj.characters, |c| c.id.clone());
        let mut remove = None;
        for (i, id) in state.proj.party.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("P{}", i + 1));
                id_combo(ui, &format!("party_{i}"), id, &char_ids);
                if ui.small_button("×").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            state.snapshot();
            state.proj.party.remove(i);
        }
        if ui.button("＋メンバー").clicked() {
            state.snapshot();
            state.proj.party.push(char_ids.first().cloned().unwrap_or_default());
        }
    });
}

// ------------------------------------------------------------------ items

fn items_tab(ctx: &egui::Context, state: &mut EditorState) {
    let labels = ids_of(&state.proj.items, |d| format!("{} {}", d.id, d.name));
    let mut sel = state.sel_item;
    egui::SidePanel::left("item_list").default_width(180.0).show(ctx, |ui| {
        match list_panel(ui, "アイテム一覧", &labels, &mut sel) {
            ListAction::Add => {
                state.snapshot();
                let ids = ids_of(&state.proj.items, |d| d.id.clone());
                state.proj.items.push(new_item(super::ops::next_id(&ids, super::ops::IdKind::Item.prefix())));
                sel = state.proj.items.len() - 1;
            }
            ListAction::Duplicate => {
                state.snapshot();
                let mut d = state.proj.items[sel].clone();
                let ids = ids_of(&state.proj.items, |d| d.id.clone());
                d.id = super::ops::next_id(&ids, super::ops::IdKind::Item.prefix());
                state.proj.items.push(d);
                sel = state.proj.items.len() - 1;
            }
            ListAction::Delete => {
                state.snapshot();
                state.proj.items.remove(sel);
                sel = sel.min(state.proj.items.len().saturating_sub(1));
            }
            ListAction::None => {}
        }
    });
    state.sel_item = sel;

    egui::CentralPanel::default().show(ctx, |ui| {
        let magic_ids = ids_of(&state.proj.magics, |d| d.id.clone());
        if state.proj.items.get(state.sel_item).is_none() {
            ui.label("アイテムを選択");
            return;
        }
        let cur_id = state.proj.items[state.sel_item].id.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            rename_row(ui, state, super::ops::IdKind::Item, cur_id);
            let mut dirty = false;
            let d = &mut state.proj.items[state.sel_item];
            dirty |= text_row(ui, "名", &mut d.name);
            ui.horizontal(|ui| {
                ui.label("種別");
                egui::ComboBox::from_id_salt("item_kind").selected_text(d.kind.label()).show_ui(ui, |ui| {
                    for k in ITEM_KINDS {
                        if ui.selectable_value(&mut d.kind, k, k.label()).changed() {
                            dirty = true;
                        }
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                dirty |= num_i(ui, "するどさ", &mut d.sharpness);
                dirty |= num_i(ui, "かたさ", &mut d.hardness);
                dirty |= num_i(ui, "おもさ", &mut d.weight);
                dirty |= num_i(ui, "温度", &mut d.temperature);
                dirty |= num_i(ui, "栄養", &mut d.nutrition);
                dirty |= num_i(ui, "対魔法", &mut d.anti_magic);
                dirty |= num_i(ui, "投げ", &mut d.throwability);
                dirty |= num_i(ui, "持ち", &mut d.grip);
                dirty |= num_u(ui, "収納", &mut d.capacity);
            });
            if ui.checkbox(&mut d.important, "だいじなもの").changed() {
                dirty = true;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("装備箇所");
                for slot in EquipSlot::ALL {
                    let mut on = d.equip_slots.contains(&slot);
                    if ui.checkbox(&mut on, slot.label()).changed() {
                        d.equip_slots.retain(|s| *s != slot);
                        if on {
                            d.equip_slots.push(slot);
                        }
                        dirty = true;
                    }
                }
            });
            ui.label("効果");
            let mut remove = None;
            for (i, e) in d.effects.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(format!("eff_{i}")).selected_text(e.stat.label()).show_ui(ui, |ui| {
                        for k in STAT_KINDS {
                            if ui.selectable_value(&mut e.stat, k, k.label()).changed() {
                                dirty = true;
                            }
                        }
                    });
                    if ui.add(egui::DragValue::new(&mut e.delta)).changed() {
                        dirty = true;
                    }
                    if ui.add(egui::DragValue::new(&mut e.duration_cycles).prefix("持続 ")).changed() {
                        dirty = true;
                    }
                    if ui.small_button("×").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                d.effects.remove(i);
                dirty = true;
            }
            if ui.button("＋効果").clicked() {
                d.effects.push(crate::item::StatEffect { stat: StatKind::Attack, delta: 0, duration_cycles: 0 });
                dirty = true;
            }
            ui.horizontal(|ui| {
                ui.label("習得魔法");
                let mut cur = d.teaches.clone().unwrap_or_default();
                if id_combo(ui, "item_teaches", &mut cur, &magic_ids) {
                    d.teaches = if cur.is_empty() { None } else { Some(cur) };
                    dirty = true;
                }
            });
            dirty |= text_row(ui, "モデル", &mut d.model);
            if dirty {
                state.touch();
            }
        });
    });
}

// ------------------------------------------------------------------ monsters

fn monsters_tab(ctx: &egui::Context, state: &mut EditorState) {
    let labels = ids_of(&state.proj.monsters, |d| format!("{} {}", d.id, d.name));
    let mut sel = state.sel_monster;
    egui::SidePanel::left("mon_list").default_width(180.0).show(ctx, |ui| {
        match list_panel(ui, "モンスター一覧", &labels, &mut sel) {
            ListAction::Add => {
                state.snapshot();
                let ids = ids_of(&state.proj.monsters, |d| d.id.clone());
                state.proj.monsters.push(new_monster(super::ops::next_id(&ids, super::ops::IdKind::Monster.prefix())));
                sel = state.proj.monsters.len() - 1;
            }
            ListAction::Duplicate => {
                state.snapshot();
                let mut d = state.proj.monsters[sel].clone();
                let ids = ids_of(&state.proj.monsters, |d| d.id.clone());
                d.id = super::ops::next_id(&ids, super::ops::IdKind::Monster.prefix());
                state.proj.monsters.push(d);
                sel = state.proj.monsters.len() - 1;
            }
            ListAction::Delete => {
                state.snapshot();
                state.proj.monsters.remove(sel);
                sel = sel.min(state.proj.monsters.len().saturating_sub(1));
            }
            ListAction::None => {}
        }
    });
    state.sel_monster = sel;

    egui::CentralPanel::default().show(ctx, |ui| {
        let item_ids = ids_of(&state.proj.items, |d| d.id.clone());
        if state.proj.monsters.get(state.sel_monster).is_none() {
            ui.label("モンスターを選択");
            return;
        }
        let cur_id = state.proj.monsters[state.sel_monster].id.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            rename_row(ui, state, super::ops::IdKind::Monster, cur_id);
            let mut dirty = false;
            let d = &mut state.proj.monsters[state.sel_monster];
            dirty |= text_row(ui, "名", &mut d.name);
            dirty |= text_row(ui, "プロフィール", &mut d.profile);
            ui.horizontal_wrapped(|ui| {
                dirty |= num_i(ui, "HP", &mut d.max_hp);
                dirty |= num_i(ui, "攻撃", &mut d.attack);
                dirty |= num_i(ui, "防御", &mut d.defense);
                dirty |= num_i(ui, "速さ", &mut d.agility);
                dirty |= num_i(ui, "対魔法", &mut d.anti_magic);
                dirty |= num_i(ui, "視界", &mut d.sight);
                dirty |= num_i(ui, "逃避HP", &mut d.flee_hp);
                dirty |= num_i(ui, "用心", &mut d.wariness);
                dirty |= num_i(ui, "経験値", &mut d.exp);
            });
            ui.horizontal_wrapped(|ui| {
                dirty |= num_u(ui, "攻撃頻度", &mut d.attack_freq);
                dirty |= num_u(ui, "行動単位", &mut d.action_unit);
                ui.label("(255=しない)");
                dirty |= num_u64(ui, "再生", &mut d.regen_cycles);
            });
            ui.horizontal_wrapped(|ui| {
                dirty |= num_i(ui, "耐空", &mut d.resist_air);
                dirty |= num_i(ui, "耐水", &mut d.resist_water);
                dirty |= num_i(ui, "耐熱", &mut d.resist_heat);
                dirty |= num_i(ui, "耐毒", &mut d.resist_poison);
                dirty |= num_i(ui, "体温", &mut d.body_temp);
            });
            ui.horizontal(|ui| {
                ui.label("移動型");
                for mt in [MoveType::Ground, MoveType::Air, MoveType::None] {
                    if ui.selectable_value(&mut d.move_type, mt, move_type_label(mt)).changed() {
                        dirty = true;
                    }
                }
            });
            if ui.checkbox(&mut d.can_use_ladder, "はしご可").changed() { dirty = true; }
            if ui.checkbox(&mut d.fits_narrow, "幅1通路可").changed() { dirty = true; }
            if ui.checkbox(&mut d.large, "大サイズ").changed() { dirty = true; }
            ui.label("所持品");
            dirty |= id_list(ui, "mon_carry", &mut d.carry_items, &item_ids);
            ui.label("攻撃用アイテム");
            dirty |= id_list(ui, "mon_attack", &mut d.attack_items, &item_ids);
            dirty |= text_row(ui, "モデル", &mut d.model);
            ui.collapsing("アニメ名", |ui| {
                dirty |= text_row(ui, "idle", &mut d.anim.idle);
                dirty |= text_row(ui, "walk", &mut d.anim.walk);
                dirty |= text_row(ui, "attack", &mut d.anim.attack);
                dirty |= text_row(ui, "hit", &mut d.anim.hit);
                dirty |= text_row(ui, "death", &mut d.anim.death);
            });
            if dirty {
                state.touch();
            }
        });
    });
}

// ------------------------------------------------------------------ magics

fn magics_tab(ctx: &egui::Context, state: &mut EditorState) {
    let labels = ids_of(&state.proj.magics, |d| format!("{} {}", d.id, d.name));
    let mut sel = state.sel_magic;
    egui::SidePanel::left("magic_list").default_width(180.0).show(ctx, |ui| {
        match list_panel(ui, "魔法一覧", &labels, &mut sel) {
            ListAction::Add => {
                state.snapshot();
                let ids = ids_of(&state.proj.magics, |d| d.id.clone());
                state.proj.magics.push(new_magic(super::ops::next_id(&ids, super::ops::IdKind::Magic.prefix())));
                sel = state.proj.magics.len() - 1;
            }
            ListAction::Duplicate => {
                state.snapshot();
                let mut d = state.proj.magics[sel].clone();
                let ids = ids_of(&state.proj.magics, |d| d.id.clone());
                d.id = super::ops::next_id(&ids, super::ops::IdKind::Magic.prefix());
                state.proj.magics.push(d);
                sel = state.proj.magics.len() - 1;
            }
            ListAction::Delete => {
                state.snapshot();
                state.proj.magics.remove(sel);
                sel = sel.min(state.proj.magics.len().saturating_sub(1));
            }
            ListAction::None => {}
        }
    });
    state.sel_magic = sel;

    egui::CentralPanel::default().show(ctx, |ui| {
        if state.proj.magics.get(state.sel_magic).is_none() {
            ui.label("魔法を選択");
            return;
        }
        let cur_id = state.proj.magics[state.sel_magic].id.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            rename_row(ui, state, super::ops::IdKind::Magic, cur_id);
            let mut dirty = false;
            let d = &mut state.proj.magics[state.sel_magic];
            dirty |= text_row(ui, "名", &mut d.name);
            dirty |= text_row(ui, "説明", &mut d.description);
            dirty |= text_row(ui, "シンボル", &mut d.symbol);
            ui.horizontal(|ui| {
                ui.label("種別");
                egui::ComboBox::from_id_salt("magic_kind").selected_text(magic_kind_label(&d.kind)).show_ui(ui, |ui| {
                    for k in magic_kind_options() {
                        if ui.selectable_value(&mut d.kind, k.clone(), magic_kind_label(&k)).changed() {
                            dirty = true;
                        }
                    }
                });
            });
            // Kind-specific parameter.
            match &mut d.kind {
                MagicKind::StatChange(stat) => {
                    egui::ComboBox::from_id_salt("magic_stat").selected_text(stat.label()).show_ui(ui, |ui| {
                        for k in STAT_KINDS {
                            if ui.selectable_value(stat, k, k.label()).changed() { dirty = true; }
                        }
                    });
                }
                MagicKind::Revive { ratio_percent } => {
                    if ui.add(egui::DragValue::new(ratio_percent).prefix("復活% ")).changed() { dirty = true; }
                }
                MagicKind::Light { strength } => {
                    if ui.add(egui::DragValue::new(strength).prefix("強さ ")).changed() { dirty = true; }
                }
                _ => {}
            }
            ui.horizontal_wrapped(|ui| {
                dirty |= num_i(ui, "MP", &mut d.mp_cost);
                dirty |= num_i(ui, "難易度", &mut d.difficulty);
                dirty |= num_i(ui, "変更値", &mut d.value);
                dirty |= num_u64(ui, "持続", &mut d.duration_cycles);
                dirty |= num_u8(ui, "光弾", &mut d.projectiles);
            });
            if ui.checkbox(&mut d.liquefiable, "液体化可").changed() { dirty = true; }
            if dirty {
                state.touch();
            }
        });
    });
}

// ------------------------------------------------------------------ events

fn events_tab(ctx: &egui::Context, state: &mut EditorState) {
    // Flat list across levels, grouped in the label.
    let mut labels = Vec::new();
    let mut index = Vec::new(); // (level, event index)
    for (li, lvl) in state.proj.levels.iter().enumerate() {
        for (ei, ev) in lvl.events.iter().enumerate() {
            labels.push(format!("L{li}: {}", ev.id));
            index.push((li, ei));
        }
    }
    let mut sel = state.sel_event.min(labels.len().saturating_sub(1));
    egui::SidePanel::left("event_list").default_width(200.0).show(ctx, |ui| {
        ui.heading("イベント一覧");
        ui.horizontal(|ui| {
            if ui.button("＋新規(現レベル)").clicked() {
                state.snapshot();
                let li = state.level_index;
                let ids = ids_of(&state.proj.levels[li].events, |e| e.id.clone());
                let id = super::ops::next_id(&ids, "event");
                state.proj.levels[li].events.push(new_event(id));
            }
            if ui.add_enabled(!index.is_empty(), egui::Button::new("削除")).clicked()
                && let Some(&(li, ei)) = index.get(sel)
            {
                state.snapshot();
                state.proj.levels[li].events.remove(ei);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                if ui.selectable_label(sel == i, label).clicked() {
                    sel = i;
                }
            }
        });
    });
    state.sel_event = sel;

    egui::CentralPanel::default().show(ctx, |ui| {
        let item_ids = ids_of(&state.proj.items, |d| d.id.clone());
        let mon_ids = ids_of(&state.proj.monsters, |d| d.id.clone());
        egui::ScrollArea::vertical().show(ui, |ui| {
            wall_texts_editor(ui, state);
            stairs_editor(ui, state);
            ui.separator();
            let Some(&(li, ei)) = index.get(sel) else {
                ui.label("イベントを選択");
                return;
            };
            let ev = &mut state.proj.levels[li].events[ei];
            let mut dirty = false;
            dirty |= text_row(ui, "id", &mut ev.id);
            ui.horizontal(|ui| {
                ui.label("座標");
                dirty |= drag_i(ui, &mut ev.at.0);
                dirty |= drag_i(ui, &mut ev.at.1);
                dirty |= drag_usize(ui, &mut ev.at.2);
            });
            dirty |= num_u64(ui, "遅延", &mut ev.delay_cycles);
            dirty |= trigger_editor(ui, &mut ev.trigger, &item_ids);
            ui.label("フラグ条件");
            ui.horizontal(|ui| {
                ui.label("結合");
                for j in [FlagJoin::And, FlagJoin::Or] {
                    if ui.selectable_value(&mut ev.join, j, flag_join_label(j)).changed() { dirty = true; }
                }
            });
            let mut fremove = None;
            for (i, fc) in ev.flags.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    if ui.add(egui::DragValue::new(&mut fc.flag).prefix("flag ")).changed() { dirty = true; }
                    if ui.checkbox(&mut fc.must_be_on, "ON").changed() { dirty = true; }
                    if ui.small_button("×").clicked() { fremove = Some(i); }
                });
            }
            if let Some(i) = fremove { ev.flags.remove(i); dirty = true; }
            if ui.button("＋条件").clicked() {
                ev.flags.push(crate::event::FlagCond { flag: 0, must_be_on: true });
                dirty = true;
            }
            ui.separator();
            ui.label("アクション列");
            dirty |= actions_editor(ui, &mut ev.actions, &item_ids, &mon_ids);
            if dirty {
                state.touch();
            }
        });
    });
}

fn wall_texts_editor(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.collapsing("書ける壁の文章 (現レベル)", |ui| {
        let li = state.level_index;
        let lvl = &mut state.proj.levels[li];
        let mut remove = None;
        let mut dirty = false;
        for (i, wt) in lvl.wall_texts.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                dirty |= drag_i(ui, &mut wt.x);
                dirty |= drag_i(ui, &mut wt.y);
                dirty |= drag_usize(ui, &mut wt.floor);
                if ui.text_edit_singleline(&mut wt.text).changed() { dirty = true; }
                if ui.small_button("×").clicked() { remove = Some(i); }
            });
        }
        if let Some(i) = remove { lvl.wall_texts.remove(i); dirty = true; }
        if ui.button("＋文章").clicked() {
            lvl.wall_texts.push(crate::project::WallText { x: 0, y: 0, floor: 0, text: String::new() });
            dirty = true;
        }
        if dirty { state.touch(); }
    });
}

fn stairs_editor(ui: &mut egui::Ui, state: &mut EditorState) {
    ui.collapsing("階段リンク (現レベル)", |ui| {
        let li = state.level_index;
        let lvl = &mut state.proj.levels[li];
        let mut remove = None;
        let mut dirty = false;
        for (i, s) in lvl.stairs_links.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label("from");
                dirty |= drag_i(ui, &mut s.from.0);
                dirty |= drag_i(ui, &mut s.from.1);
                dirty |= drag_usize(ui, &mut s.from.2);
                ui.label("→ L");
                dirty |= drag_usize(ui, &mut s.to_level);
                dirty |= drag_i(ui, &mut s.to.0);
                dirty |= drag_i(ui, &mut s.to.1);
                dirty |= drag_usize(ui, &mut s.to.2);
                if ui.small_button("×").clicked() { remove = Some(i); }
            });
        }
        if let Some(i) = remove { lvl.stairs_links.remove(i); dirty = true; }
        if ui.button("＋リンク").clicked() {
            lvl.stairs_links.push(crate::project::StairsLink { from: (0, 0, 0), to_level: 0, to: (0, 0, 0), to_facing: Facing::North });
            dirty = true;
        }
        if dirty { state.touch(); }
    });
}

fn trigger_editor(ui: &mut egui::Ui, trigger: &mut TriggerKind, item_ids: &[String]) -> bool {
    let mut dirty = false;
    ui.horizontal(|ui| {
        ui.label("トリガー");
        egui::ComboBox::from_id_salt("trigger_kind").selected_text(trigger_label(trigger)).show_ui(ui, |ui| {
            for t in trigger_options() {
                if ui.selectable_value(trigger, t.clone(), trigger_label(&t)).changed() { dirty = true; }
            }
        });
    });
    match trigger {
        TriggerKind::Keyhole { key_item } => {
            if id_combo(ui, "trig_key", key_item, item_ids) { dirty = true; }
        }
        TriggerKind::WarpPoint { hidden } => {
            if ui.checkbox(hidden, "隠し").changed() { dirty = true; }
        }
        TriggerKind::FloorPlate { cond } => {
            egui::ComboBox::from_id_salt("plate_cond").selected_text(plate_cond_label(cond)).show_ui(ui, |ui| {
                for c in [PlateCond::Step, PlateCond::Weight { min_x100g: 0 }, PlateCond::ItemPlaced { item: None }] {
                    if ui.selectable_value(cond, c.clone(), plate_cond_label(&c)).changed() { dirty = true; }
                }
            });
            if let PlateCond::Weight { min_x100g } = cond
                && ui.add(egui::DragValue::new(min_x100g).prefix("重量 ")).changed() { dirty = true; }
        }
        _ => {}
    }
    dirty
}

fn actions_editor(ui: &mut egui::Ui, actions: &mut Vec<EventAction>, item_ids: &[String], mon_ids: &[String]) -> bool {
    let mut dirty = false;
    let mut remove = None;
    let mut move_up = None;
    for (i, a) in actions.iter_mut().enumerate() {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(format!("act_{i}")).selected_text(action_label(a)).show_ui(ui, |ui| {
                    for opt in action_options() {
                        if ui.selectable_value(a, opt.clone(), action_label(&opt)).changed() { dirty = true; }
                    }
                });
                if ui.small_button("↑").clicked() { move_up = Some(i); }
                if ui.small_button("×").clicked() { remove = Some(i); }
            });
            match a {
                EventAction::SetFlag { flag, on } => {
                    if ui.add(egui::DragValue::new(flag).prefix("flag ")).changed() { dirty = true; }
                    if ui.checkbox(on, "ON").changed() { dirty = true; }
                }
                EventAction::SetDoor { kind, open } => {
                    if ui.add(egui::DragValue::new(kind).prefix("kind ")).changed() { dirty = true; }
                    if ui.checkbox(open, "開").changed() { dirty = true; }
                }
                EventAction::SpawnItem { item, x, y, floor } => {
                    if id_combo(ui, format!("aitem_{i}").as_str(), item, item_ids) { dirty = true; }
                    dirty |= drag_i(ui, x); dirty |= drag_i(ui, y); dirty |= drag_usize(ui, floor);
                }
                EventAction::SpawnMonster { monster, x, y, floor } => {
                    if id_combo(ui, format!("amon_{i}").as_str(), monster, mon_ids) { dirty = true; }
                    dirty |= drag_i(ui, x); dirty |= drag_i(ui, y); dirty |= drag_usize(ui, floor);
                }
                EventAction::Warp { level, x, y, floor, .. } => {
                    dirty |= drag_usize(ui, level); dirty |= drag_i(ui, x); dirty |= drag_i(ui, y); dirty |= drag_usize(ui, floor);
                }
                EventAction::SetBlock { x, y, floor, .. } => {
                    dirty |= drag_i(ui, x); dirty |= drag_i(ui, y); dirty |= drag_usize(ui, floor);
                }
                EventAction::SetLiquid { x, y, floor, kind } => {
                    dirty |= drag_i(ui, x); dirty |= drag_i(ui, y); dirty |= drag_usize(ui, floor);
                    let mut is_some = kind.is_some();
                    if ui.checkbox(&mut is_some, "液体").changed() {
                        *kind = if is_some { Some(LiquidKind::Water) } else { None };
                        dirty = true;
                    }
                }
                EventAction::SetMoveMode { mode } => {
                    for m in [MoveMode::Normal, MoveMode::Free, MoveMode::Locked] {
                        if ui.selectable_value(mode, m, move_mode_label(m)).changed() { dirty = true; }
                    }
                }
                _ => {}
            }
        });
    }
    if let Some(i) = remove { actions.remove(i); dirty = true; }
    if let Some(i) = move_up
        && i > 0 { actions.swap(i, i - 1); dirty = true; }
    if ui.button("＋アクション").clicked() {
        actions.push(EventAction::SetFlag { flag: 0, on: true });
        dirty = true;
    }
    dirty
}

// ------------------------------------------------------------------ settings

fn settings_tab(ctx: &egui::Context, state: &mut EditorState) {
    egui::CentralPanel::default().show(ctx, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut dirty = false;
            dirty |= text_row(ui, "プロジェクト名", &mut state.proj.name);
            ui.separator();
            ui.heading("上限 (LimitsConfig)");
            let l = &mut state.proj.limits;
            ui.horizontal_wrapped(|ui| {
                dirty |= num_usize(ui, "レベル数", &mut l.max_levels);
                dirty |= num_usize(ui, "フロア/レベル", &mut l.floors_per_level);
                dirty |= num_usize(ui, "幅", &mut l.floor_width);
                dirty |= num_usize(ui, "高さ", &mut l.floor_height);
                dirty |= num_usize(ui, "キャラ数", &mut l.max_characters);
                dirty |= num_usize(ui, "パーティ", &mut l.party_size);
                dirty |= num_usize(ui, "アイテム種", &mut l.max_item_kinds);
                dirty |= num_usize(ui, "モンスター種", &mut l.max_monster_kinds);
                dirty |= num_usize(ui, "魔法種", &mut l.max_magic_kinds);
                dirty |= num_usize(ui, "フラグ数", &mut l.event_flags);
            });
            ui.separator();
            ui.heading("ゲームルール");
            let h = &mut state.proj.rules.hunger;
            if ui.checkbox(&mut h.enabled, "空腹度 有効").changed() { dirty = true; }
            ui.horizontal_wrapped(|ui| {
                dirty |= num_i(ui, "満腹最大", &mut h.satiety_max);
                dirty |= num_u64(ui, "減少間隔", &mut h.drain_interval_cycles);
                dirty |= num_i(ui, "餓死ダメージ", &mut h.starvation_damage);
                dirty |= num_i(ui, "栄養係数", &mut h.satiety_per_nutrition);
            });
            ui.separator();
            ui.heading("イベントフラグ初期値");
            let cols = 16;
            let total = state.proj.limits.event_flags;
            egui::Grid::new("flags").show(ui, |ui| {
                for i in 0..total {
                    let mut on = state.proj.initial_flags.contains(&i);
                    if ui.checkbox(&mut on, format!("{i}")).changed() {
                        state.proj.initial_flags.retain(|&f| f != i);
                        if on {
                            state.proj.initial_flags.push(i);
                        }
                        dirty = true;
                    }
                    if (i + 1) % cols == 0 {
                        ui.end_row();
                    }
                }
            });
            if dirty {
                state.touch();
                state.recompute_warnings();
            }
        });
    });
}

// ------------------------------------------------------------------ small widgets

fn text_row(ui: &mut egui::Ui, label: &str, s: &mut String) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        if ui.text_edit_singleline(s).changed() {
            changed = true;
        }
    });
    changed
}

fn num_i(ui: &mut egui::Ui, label: &str, v: &mut i32) -> bool {
    ui.add(egui::DragValue::new(v).prefix(format!("{label} "))).changed()
}
fn num_u(ui: &mut egui::Ui, label: &str, v: &mut u32) -> bool {
    ui.add(egui::DragValue::new(v).prefix(format!("{label} "))).changed()
}
fn num_u8(ui: &mut egui::Ui, label: &str, v: &mut u8) -> bool {
    ui.add(egui::DragValue::new(v).prefix(format!("{label} "))).changed()
}
fn num_u64(ui: &mut egui::Ui, label: &str, v: &mut u64) -> bool {
    ui.add(egui::DragValue::new(v).prefix(format!("{label} "))).changed()
}
fn num_usize(ui: &mut egui::Ui, label: &str, v: &mut usize) -> bool {
    ui.add(egui::DragValue::new(v).prefix(format!("{label} "))).changed()
}
fn num_row(ui: &mut egui::Ui, label: &str, v: &mut f32) -> bool {
    ui.add(egui::DragValue::new(v).prefix(format!("{label} "))).changed()
}
fn drag_i(ui: &mut egui::Ui, v: &mut i32) -> bool {
    ui.add(egui::DragValue::new(v)).changed()
}
fn drag_usize(ui: &mut egui::Ui, v: &mut usize) -> bool {
    ui.add(egui::DragValue::new(v)).changed()
}

/// An id label + rename box: typing a new id and pressing 改名 propagates the
/// rename across every reference (ops::rename_id) and reports how many updated.
fn rename_row(ui: &mut egui::Ui, state: &mut EditorState, kind: super::ops::IdKind, cur_id: String) {
    ui.horizontal(|ui| {
        ui.label(format!("id: {cur_id}"));
        ui.text_edit_singleline(&mut state.rename_buf);
        if ui.button("改名").clicked() {
            let new = state.rename_buf.trim().to_string();
            if !new.is_empty() && new != cur_id {
                state.snapshot();
                let n = super::ops::rename_id(&mut state.proj, kind, &cur_id, &new);
                state.status = format!("{}: {n}件の参照を更新", kind.label());
                state.recompute_warnings();
            }
            state.rename_buf.clear();
        }
    });
}

// ------------------------------------------------------------------ new instances

fn new_character(id: String) -> crate::character::Character {
    use crate::character::{Character, GrowthType, Stats};
    Character {
        id,
        first_name: "なまえ".into(),
        last_name: String::new(),
        gender: String::new(),
        height_cm: 170.0,
        weight_kg: 60.0,
        birth_date: "1000-01-01".into(),
        age: 20,
        likes: String::new(),
        dislikes: String::new(),
        background: String::new(),
        growth: GrowthType::Average,
        stats: Stats {
            level: 1, max_hp: 30, max_mp: 10, attack: 5, defense: 5, agility: 5, throwing: 5,
            carrying: 10, lung_capacity: 5, heat_resist: 5, poison_resist: 5, magic_knowledge: 5,
            concentration: 20, appraisal: 5, stealing: 5, bite: 5,
        },
        model: "models/party/knight.glb".into(),
        portrait: String::new(),
        items: Vec::new(),
        magics: Vec::new(),
    }
}

fn new_item(id: String) -> crate::item::ItemDef {
    crate::item::ItemDef {
        id,
        name: "あたらしい道具".into(),
        kind: ItemKind::General,
        sharpness: 0, hardness: 0, weight: 1, temperature: 20, nutrition: 0, entropy_max: 0,
        anti_magic: 0, anti_appraisal: 0, anti_impact: 0, important: false, throwability: 0,
        grip: 0, capacity: 0, equip_slots: Vec::new(), effects: Vec::new(), model: String::new(),
        teaches: None,
    }
}

fn new_monster(id: String) -> crate::monster::MonsterDef {
    use crate::monster::{MonsterAnims, MonsterDef, MoveType};
    MonsterDef {
        id,
        name: "あたらしい敵".into(),
        profile: String::new(),
        max_hp: 20, attack: 5, defense: 2, agility: 5, attack_freq: 20, anti_magic: 0, body_temp: 0,
        resist_air: 0, resist_water: 0, resist_heat: 0, resist_poison: 0, wariness: 0, regen_cycles: 0,
        move_type: MoveType::Ground, can_use_ladder: false, fits_narrow: true, sight: 5, action_unit: 16,
        flee_hp: 0, large: false, carry_items: Vec::new(), attack_items: Vec::new(), exp: 5,
        model: "models/enemies/skeleton_minion.glb".into(),
        anim: MonsterAnims { idle: "Idle".into(), walk: "Walking_A".into(), attack: "1H_Melee_Attack_Slice_Diagonal".into(), hit: "Hit_A".into(), death: "Death_A".into() },
    }
}

fn new_magic(id: String) -> crate::magic::MagicDef {
    crate::magic::MagicDef {
        id,
        name: "あたらしい魔法".into(),
        description: String::new(),
        mp_cost: 5, difficulty: 5, kind: MagicKind::HpChange, value: 10, duration_cycles: 0,
        liquefiable: false, projectiles: 0, symbol: "◇".into(),
    }
}

fn new_event(id: String) -> crate::event::EventDef {
    crate::event::EventDef {
        id,
        trigger: TriggerKind::SwitchPush,
        at: (0, 0, 0),
        delay_cycles: 0,
        flags: Vec::new(),
        join: FlagJoin::And,
        actions: Vec::new(),
    }
}

// ------------------------------------------------------------------ enum options / labels

const ITEM_KINDS: [ItemKind; 14] = [
    ItemKind::General, ItemKind::Scroll, ItemKind::EmptyContainer, ItemKind::Liquid, ItemKind::Light,
    ItemKind::Map, ItemKind::Compass, ItemKind::Periscope, ItemKind::Pencil, ItemKind::RedPencil,
    ItemKind::BluePencil, ItemKind::Accessory, ItemKind::GlowStone, ItemKind::TreasureChest,
];

const STAT_KINDS: [StatKind; 15] = [
    StatKind::MaxHp, StatKind::MaxMp, StatKind::Attack, StatKind::Defense, StatKind::Agility,
    StatKind::Throwing, StatKind::Carrying, StatKind::LungCapacity, StatKind::HeatResist,
    StatKind::PoisonResist, StatKind::MagicKnowledge, StatKind::Concentration, StatKind::Appraisal,
    StatKind::Stealing, StatKind::Bite,
];

fn stat_fields(s: &mut crate::character::Stats) -> Vec<(&'static str, &mut i32)> {
    vec![
        ("最大HP", &mut s.max_hp), ("最大MP", &mut s.max_mp), ("攻撃", &mut s.attack),
        ("防御", &mut s.defense), ("速さ", &mut s.agility), ("投げ", &mut s.throwing),
        ("運搬", &mut s.carrying), ("肺", &mut s.lung_capacity), ("耐熱", &mut s.heat_resist),
        ("耐毒", &mut s.poison_resist), ("魔法知識", &mut s.magic_knowledge), ("集中", &mut s.concentration),
        ("鑑定", &mut s.appraisal), ("盗み", &mut s.stealing), ("歯", &mut s.bite),
    ]
}

fn magic_kind_options() -> Vec<MagicKind> {
    vec![
        MagicKind::HpChange,
        MagicKind::MpChange,
        MagicKind::StatChange(StatKind::Defense),
        MagicKind::Revive { ratio_percent: 50 },
        MagicKind::Light { strength: 2 },
    ]
}
fn magic_kind_label(k: &MagicKind) -> String {
    match k {
        MagicKind::HpChange => "HP変化".into(),
        MagicKind::MpChange => "MP変化".into(),
        MagicKind::StatChange(_) => "能力値変化".into(),
        MagicKind::Revive { .. } => "復活".into(),
        MagicKind::Light { .. } => "照明".into(),
    }
}

fn trigger_options() -> Vec<TriggerKind> {
    vec![
        TriggerKind::SwitchPush,
        TriggerKind::SwitchToggle,
        TriggerKind::SwitchOneWay,
        TriggerKind::Keyhole { key_item: String::new() },
        TriggerKind::FloorPlate { cond: PlateCond::Step },
        TriggerKind::WarpPoint { hidden: false },
        TriggerKind::None,
    ]
}
fn trigger_label(t: &TriggerKind) -> String {
    match t {
        TriggerKind::SwitchPush => "スイッチ(押)".into(),
        TriggerKind::SwitchToggle => "スイッチ(トグル)".into(),
        TriggerKind::SwitchOneWay => "スイッチ(一度)".into(),
        TriggerKind::Keyhole { .. } => "鍵穴".into(),
        TriggerKind::FloorPlate { .. } => "しかけ床".into(),
        TriggerKind::WarpPoint { .. } => "ワープ".into(),
        TriggerKind::None => "なし".into(),
    }
}

fn action_options() -> Vec<EventAction> {
    vec![
        EventAction::SetFlag { flag: 0, on: true },
        EventAction::SetDoor { kind: 0, open: true },
        EventAction::SpawnItem { item: String::new(), x: 0, y: 0, floor: 0 },
        EventAction::SpawnMonster { monster: String::new(), x: 0, y: 0, floor: 0 },
        EventAction::Warp { level: 0, x: 0, y: 0, floor: 0, facing: Facing::North },
        EventAction::SetBlock { x: 0, y: 0, floor: 0, block: Block::Wall },
        EventAction::SetLiquid { x: 0, y: 0, floor: 0, kind: Some(LiquidKind::Water) },
        EventAction::ReviveParty,
        EventAction::SetMoveMode { mode: MoveMode::Normal },
        EventAction::ChangeBgm { bgm: String::new() },
        EventAction::StartDemo { demo: String::new() },
        EventAction::EndChain,
        EventAction::Loop,
    ]
}
fn action_label(a: &EventAction) -> String {
    match a {
        EventAction::SetFlag { .. } => "フラグ設定".into(),
        EventAction::SetDoor { .. } => "ドア開閉".into(),
        EventAction::SpawnItem { .. } => "アイテム発生".into(),
        EventAction::SpawnMonster { .. } => "モンスター発生".into(),
        EventAction::Warp { .. } => "ワープ".into(),
        EventAction::SetBlock { .. } => "ブロック発生".into(),
        EventAction::SetLiquid { .. } => "水位変更".into(),
        EventAction::ReviveParty => "パーティ復活".into(),
        EventAction::SetMoveMode { .. } => "移動モード".into(),
        EventAction::ChangeBgm { .. } => "BGM変更".into(),
        EventAction::StartDemo { .. } => "デモ起動".into(),
        EventAction::OperateSwitch { .. } => "スイッチ操作".into(),
        EventAction::EndChain => "連結終了".into(),
        EventAction::Loop => "ループ".into(),
    }
}

// ------------------------------------------------------------------ block visuals (from plan3)

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
        Block::Hole => egui::Color32::from_rgb(10, 10, 14),
        Block::Stairs { .. } => egui::Color32::from_rgb(150, 120, 70),
        Block::WritableWall => egui::Color32::from_rgb(110, 100, 90),
        Block::HoroscopeVert { .. } => egui::Color32::from_rgb(120, 90, 200),
        Block::Keyhole => egui::Color32::from_rgb(180, 160, 40),
        Block::Switch => egui::Color32::from_rgb(200, 60, 60),
        Block::FloorPlate => egui::Color32::from_rgb(120, 130, 150),
        Block::WarpPoint => egui::Color32::from_rgb(90, 200, 200),
    }
}

fn cell_glyph(block: Block) -> Option<char> {
    match block {
        Block::Ladder => Some('H'),
        Block::Door { kind: 0 } => Some('1'),
        Block::Door { .. } => Some('2'),
        Block::Horoscope { pass_from } => Some(match pass_from {
            Facing::West => '<', Facing::East => '>', Facing::North => '^', Facing::South => 'v',
        }),
        Block::Hole => Some('o'),
        Block::Stairs { up: true } => Some('u'),
        Block::Stairs { up: false } => Some('d'),
        Block::WritableWall => Some('W'),
        Block::HoroscopeVert { from_below: true } => Some('A'),
        Block::HoroscopeVert { from_below: false } => Some('V'),
        Block::Keyhole => Some('K'),
        Block::Switch => Some('S'),
        Block::FloorPlate => Some('P'),
        Block::WarpPoint => Some('T'),
        _ => None,
    }
}

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
