//! Editor content operations that are independent of egui, so they can be unit
//! tested: id auto-numbering, id-rename **reference propagation** (the single
//! source of truth for "where is an id referenced"), project validation, and
//! level-delete reference warnings.
//!
//! plan9 note: the propagation list here is deliberately the one place that
//! enumerates cross-references; later plans that add reference sites update this
//! function (and its test) rather than scattering the knowledge.

use crate::event::{EventAction, PlateCond, TriggerKind};
use crate::project::Project;

/// A referenceable content kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdKind {
    Character,
    Item,
    Monster,
    Magic,
}

impl IdKind {
    pub fn prefix(self) -> &'static str {
        match self {
            IdKind::Character => "char",
            IdKind::Item => "item",
            IdKind::Monster => "mon",
            IdKind::Magic => "magic",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            IdKind::Character => "キャラ",
            IdKind::Item => "アイテム",
            IdKind::Monster => "モンスター",
            IdKind::Magic => "魔法",
        }
    }
}

/// The next auto-numbered id for `prefix` (e.g. `item_007`), unique in `existing`.
pub fn next_id(existing: &[String], prefix: &str) -> String {
    for n in 1.. {
        let candidate = format!("{prefix}_{n:03}");
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
    }
    unreachable!("1.. is unbounded")
}

/// Rename `old` → `new` for `kind`, updating the definition and every reference
/// across the whole project. Returns the count of references updated (not
/// counting the definition itself). No-op returning 0 if `old == new`.
pub fn rename_id(project: &mut Project, kind: IdKind, old: &str, new: &str) -> usize {
    if old == new {
        return 0;
    }
    let mut n = 0;
    let mut swap = |s: &mut String| {
        if s == old {
            *s = new.to_string();
            n += 1;
        }
    };

    match kind {
        IdKind::Character => {
            if let Some(c) = project.characters.iter_mut().find(|c| c.id == old) {
                c.id = new.to_string();
            }
            for id in &mut project.party {
                swap(id);
            }
        }
        IdKind::Item => {
            if let Some(d) = project.items.iter_mut().find(|d| d.id == old) {
                d.id = new.to_string();
            }
            for c in &mut project.characters {
                for id in &mut c.items {
                    swap(id);
                }
            }
            for m in &mut project.monsters {
                for id in m.carry_items.iter_mut().chain(m.attack_items.iter_mut()) {
                    swap(id);
                }
            }
            for lvl in &mut project.levels {
                for p in &mut lvl.items {
                    swap(&mut p.id);
                }
                for ev in &mut lvl.events {
                    if let TriggerKind::Keyhole { key_item } = &mut ev.trigger {
                        swap(key_item);
                    }
                    if let TriggerKind::FloorPlate { cond: PlateCond::ItemPlaced { item: Some(item) } } = &mut ev.trigger {
                        swap(item);
                    }
                    for a in &mut ev.actions {
                        if let EventAction::SpawnItem { item, .. } = a {
                            swap(item);
                        }
                    }
                }
            }
        }
        IdKind::Monster => {
            if let Some(d) = project.monsters.iter_mut().find(|d| d.id == old) {
                d.id = new.to_string();
            }
            for lvl in &mut project.levels {
                for p in &mut lvl.monsters {
                    swap(&mut p.id);
                }
                for ev in &mut lvl.events {
                    for a in &mut ev.actions {
                        if let EventAction::SpawnMonster { monster, .. } = a {
                            swap(monster);
                        }
                    }
                }
            }
        }
        IdKind::Magic => {
            if let Some(d) = project.magics.iter_mut().find(|d| d.id == old) {
                d.id = new.to_string();
            }
            for c in &mut project.characters {
                for id in &mut c.magics {
                    swap(id);
                }
            }
            for d in &mut project.items {
                if let Some(m) = &mut d.teaches {
                    swap(m);
                }
            }
        }
    }
    n
}

/// Non-fatal validation warnings (dangling references + limit overages +
/// trigger/event coordinate mismatches). Recomputed on save / tab switch.
pub fn validate(project: &Project) -> Vec<String> {
    let mut w = Vec::new();
    let l = &project.limits;
    let has = |v: &[String], id: &str| v.iter().any(|x| x == id);
    let char_ids: Vec<String> = project.characters.iter().map(|c| c.id.clone()).collect();
    let item_ids: Vec<String> = project.items.iter().map(|d| d.id.clone()).collect();
    let mon_ids: Vec<String> = project.monsters.iter().map(|d| d.id.clone()).collect();
    let magic_ids: Vec<String> = project.magics.iter().map(|d| d.id.clone()).collect();

    // Limit overages.
    let over = |w: &mut Vec<String>, n: usize, max: usize, what: &str| {
        if n > max {
            w.push(format!("{what}が上限超過: {n} / {max}"));
        }
    };
    over(&mut w, project.characters.len(), l.max_characters, "キャラ数");
    over(&mut w, project.party.len(), l.party_size, "パーティ人数");
    over(&mut w, project.items.len(), l.max_item_kinds, "アイテム種類");
    over(&mut w, project.monsters.len(), l.max_monster_kinds, "モンスター種類");
    over(&mut w, project.magics.len(), l.max_magic_kinds, "魔法種類");
    over(&mut w, project.levels.len(), l.max_levels, "レベル数");

    // Dangling references.
    for id in &project.party {
        if !has(&char_ids, id) {
            w.push(format!("party: 未定義キャラ '{id}'"));
        }
    }
    for c in &project.characters {
        for id in &c.items {
            if !has(&item_ids, id) {
                w.push(format!("{}: 未定義アイテム '{id}'", c.id));
            }
        }
        for id in &c.magics {
            if !has(&magic_ids, id) {
                w.push(format!("{}: 未定義魔法 '{id}'", c.id));
            }
        }
    }
    for d in &project.items {
        if let Some(m) = &d.teaches
            && !has(&magic_ids, m)
        {
            w.push(format!("{}: 未定義魔法を教える '{m}'", d.id));
        }
        if !d.effects.is_empty() && d.equip_slots.is_empty() && d.nutrition == 0 {
            w.push(format!("{}: 効果があるが装備箇所も栄養価もない", d.id));
        }
    }
    for m in &project.monsters {
        for id in m.carry_items.iter().chain(&m.attack_items) {
            if !has(&item_ids, id) {
                w.push(format!("{}: 未定義アイテム '{id}'", m.id));
            }
        }
    }
    for (li, lvl) in project.levels.iter().enumerate() {
        over(&mut w, lvl.items.len(), l.item_placements_per_level, &format!("level{li} アイテム配置"));
        over(&mut w, lvl.monsters.len(), l.monster_placements_per_level, &format!("level{li} モンスター配置"));
        for p in &lvl.items {
            if !has(&item_ids, &p.id) {
                w.push(format!("level{li}: 未定義アイテム配置 '{}'", p.id));
            }
        }
        for p in &lvl.monsters {
            if !has(&mon_ids, &p.id) {
                w.push(format!("level{li}: 未定義モンスター配置 '{}'", p.id));
            }
        }
        for tm in trigger_event_mismatches(lvl) {
            w.push(format!("level{li}: {tm}"));
        }
    }

    // Demos (plan10): count/line limits, duplicate ids, dangling StartDemo refs.
    over(&mut w, project.demos.len(), l.max_demos, "デモ本数");
    let demo_ids: Vec<String> = project.demos.iter().map(|d| d.id.clone()).collect();
    for (i, d) in project.demos.iter().enumerate() {
        if d.id.is_empty() {
            w.push(format!("demo[{i}]: idが空"));
        } else if demo_ids.iter().filter(|x| **x == d.id).count() > 1 {
            w.push(format!("demo id重複 '{}'", d.id));
        }
        over(&mut w, d.lines.len(), l.demo_message_lines, &format!("デモ'{}' 行数", d.id));
    }
    for (li, lvl) in project.levels.iter().enumerate() {
        for ev in &lvl.events {
            for a in &ev.actions {
                if let crate::event::EventAction::StartDemo { demo } = a
                    && !has(&demo_ids, demo)
                {
                    w.push(format!("level{li} {}: 未定義デモ '{demo}'", ev.id));
                }
            }
        }
    }
    w
}

/// Warnings for trigger blocks whose coordinate has no matching `EventDef`.
pub fn trigger_event_mismatches(lvl: &crate::project::LevelData) -> Vec<String> {
    use crate::dungeon::Block;
    let mut out = Vec::new();
    let is_trigger = |b: Block| matches!(b, Block::Keyhole | Block::Switch | Block::FloorPlate | Block::WarpPoint);
    for f in 0..lvl.level.floor_count() {
        let Some(floor) = lvl.level.floor(f) else { continue };
        for y in 0..floor.height {
            for x in 0..floor.width {
                let (xi, yi) = (x as i32, y as i32);
                if floor.get(xi, yi).is_some_and(is_trigger)
                    && !lvl.events.iter().any(|e| e.at == (xi, yi, f))
                {
                    out.push(format!("トリガーブロック ({xi},{yi},f{f}) にイベント未設定"));
                }
            }
        }
    }
    out
}

/// Warnings raised by deleting level `index`: stairs links and warp actions that
/// point at it (their targets would dangle / renumber).
pub fn level_delete_warnings(project: &Project, index: usize) -> Vec<String> {
    let mut w = Vec::new();
    for (li, lvl) in project.levels.iter().enumerate() {
        if li == index {
            continue;
        }
        for s in &lvl.stairs_links {
            if s.to_level == index {
                w.push(format!("level{li} の階段が level{index} を参照"));
            }
        }
        for ev in &lvl.events {
            for a in &ev.actions {
                if let EventAction::Warp { level, .. } = a
                    && *level == index
                {
                    w.push(format!("level{li} のワープ '{}' が level{index} を参照", ev.id));
                }
            }
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Project {
        crate::project::load_project("assets/projects/sample").expect("sample loads")
    }

    #[test]
    fn auto_numbering_skips_used_ids() {
        let existing = vec!["item_001".to_string(), "item_002".to_string()];
        assert_eq!(next_id(&existing, "item"), "item_003");
        assert_eq!(next_id(&[], "mon"), "mon_001");
    }

    #[test]
    fn rename_propagates_across_references() {
        let mut p = sample();
        // "key_bronze" is referenced by a keyhole event and (via placement?) —
        // rename it and count the updated references.
        let n = rename_id(&mut p, IdKind::Item, "key_bronze", "key_gold");
        assert!(n >= 1, "expected key_bronze references to be updated, got {n}");
        // The definition itself moved.
        assert!(p.items.iter().any(|d| d.id == "key_gold"));
        assert!(!p.items.iter().any(|d| d.id == "key_bronze"));
        // No dangling "key_bronze" references remain in the keyhole trigger.
        let dangling = p.levels.iter().any(|l| {
            l.events.iter().any(|e| matches!(&e.trigger, TriggerKind::Keyhole { key_item } if key_item == "key_bronze"))
        });
        assert!(!dangling);
    }

    #[test]
    fn rename_magic_updates_character_and_scroll() {
        let mut p = sample();
        let n = rename_id(&mut p, IdKind::Magic, "heal", "cure");
        // ソロン knows heal; scroll_heal teaches heal → at least 2 references.
        assert!(n >= 2, "expected heal references updated, got {n}");
        assert!(p.characters.iter().any(|c| c.magics.iter().any(|m| m == "cure")));
        assert!(p.items.iter().any(|d| d.teaches.as_deref() == Some("cure")));
    }

    #[test]
    fn validate_flags_overage_and_dangling() {
        let mut p = sample();
        // Clean sample should have no dangling-reference warnings.
        assert!(validate(&p).iter().all(|w| !w.contains("未定義")), "{:?}", validate(&p));
        // Break a reference and lower a limit.
        p.party.push("ghost".into());
        p.limits.max_item_kinds = 1;
        let w = validate(&p);
        assert!(w.iter().any(|s| s.contains("未定義キャラ 'ghost'")));
        assert!(w.iter().any(|s| s.contains("アイテム種類が上限超過")));
    }

    #[test]
    fn level_delete_warns_on_references() {
        let p = sample();
        // level00 stairs link to level 1; deleting level 1 must warn.
        let w = level_delete_warnings(&p, 1);
        assert!(w.iter().any(|s| s.contains("level1")), "{w:?}");
    }
}
