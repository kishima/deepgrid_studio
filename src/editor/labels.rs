//! Japanese display names for every enum the editor shows (2026-07 feedback:
//! 名前は日本語に統一). This module is the single place UI wording for data
//! enums lives — swapping the editor to English later means replacing these
//! functions (or backing them with a lookup table), not hunting through ui.rs.
//! Trigger / event-action / magic-kind labels stay in ui.rs beside their
//! `*_options()` builders; item kinds and stats use the in-game `label()`s.

use crate::character::GrowthType;
use crate::dungeon::{Block, Facing};
use crate::event::{FlagJoin, MoveMode, PlateCond};
use crate::monster::MoveType;

/// Palette / erase names for map blocks (the 2D and 3D editors share these).
pub fn block_label(block: Block) -> &'static str {
    match block {
        Block::Wall => "壁",
        Block::Empty => "空間",
        Block::Water => "水",
        Block::Fire => "火",
        Block::Poison => "毒",
        Block::Ladder => "はしご",
        Block::Door { kind: 0 } => "ドア1",
        Block::Door { .. } => "ドア2",
        Block::Horoscope { pass_from: Facing::West } => "ホロ(西から)",
        Block::Horoscope { pass_from: Facing::East } => "ホロ(東から)",
        Block::Horoscope { pass_from: Facing::North } => "ホロ(北から)",
        Block::Horoscope { pass_from: Facing::South } => "ホロ(南から)",
        Block::Hole => "穴",
        Block::Stairs { up: true } => "上り階段",
        Block::Stairs { up: false } => "下り階段",
        Block::WritableWall => "書ける壁",
        Block::HoroscopeVert { from_below: true } => "縦ホロ(上り)",
        Block::HoroscopeVert { from_below: false } => "縦ホロ(下り)",
        Block::Keyhole => "鍵穴",
        Block::Switch => "スイッチ",
        Block::FloorPlate => "しかけ床",
        Block::WarpPoint => "ワープ",
    }
}

/// Growth types, worded as in dandan_spec_things_editor.md.
pub fn growth_label(g: GrowthType) -> &'static str {
    match g {
        GrowthType::Average => "平均型",
        GrowthType::EarlyBloomer => "早期開花型",
        GrowthType::LateBloomer => "大器晩成型",
        GrowthType::Genius => "天才型",
        GrowthType::Talentless => "才能なし",
    }
}

pub fn move_type_label(mt: MoveType) -> &'static str {
    match mt {
        MoveType::Ground => "地上",
        MoveType::Air => "空中",
        MoveType::None => "移動しない",
    }
}

pub fn flag_join_label(j: FlagJoin) -> &'static str {
    match j {
        FlagJoin::And => "すべて(AND)",
        FlagJoin::Or => "いずれか(OR)",
    }
}

pub fn plate_cond_label(c: &PlateCond) -> &'static str {
    match c {
        PlateCond::Step => "踏む",
        PlateCond::Weight { .. } => "重量",
        PlateCond::ItemPlaced { .. } => "アイテム設置",
    }
}

pub fn move_mode_label(m: MoveMode) -> &'static str {
    match m {
        MoveMode::Normal => "通常",
        MoveMode::Free => "自由移動",
        MoveMode::Locked => "移動禁止",
    }
}
