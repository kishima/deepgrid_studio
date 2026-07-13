//! Item data model (plan5, dandan_spec_things_editor.md「Item」).
//!
//! Split, like characters, into an immutable **definition** (`ItemDef`, loaded
//! from `items.ron`) and a per-play **instance** (`ItemInstance`). An `Inventory`
//! holds instances across hands / equipment / pouch / backpack; passive stat
//! effects come only from the six equipment slots.
//!
//! Original number ranges (weights, ±127 effect deltas, etc.) are reference
//! values and are not enforced — wide `i32` fields (project.md「上限値の扱い」).

use std::collections::HashMap;

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

use crate::character::StatKind;

/// Item category (dandan_spec: 14 kinds). Kind-specific behaviour lands across
/// later plans, but every kind is defined now so data can reference it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemKind {
    General,
    Scroll,
    EmptyContainer,
    Liquid,
    Light,
    Map,
    Compass,
    Periscope,
    Pencil,
    RedPencil,
    BluePencil,
    Accessory,
    GlowStone,
    TreasureChest,
}

impl ItemKind {
    /// Japanese label for the data screen.
    pub fn label(self) -> &'static str {
        match self {
            ItemKind::General => "一般",
            ItemKind::Scroll => "巻物",
            ItemKind::EmptyContainer => "容器",
            ItemKind::Liquid => "液体",
            ItemKind::Light => "照明",
            ItemKind::Map => "地図",
            ItemKind::Compass => "コンパス",
            ItemKind::Periscope => "潜望鏡",
            ItemKind::Pencil => "鉛筆",
            ItemKind::RedPencil => "赤鉛筆",
            ItemKind::BluePencil => "青鉛筆",
            ItemKind::Accessory => "装飾品",
            ItemKind::GlowStone => "発光石",
            ItemKind::TreasureChest => "宝箱",
        }
    }

    /// Tint used by the generic (no-model) floor display so kinds read apart.
    pub fn color(self) -> (f32, f32, f32) {
        match self {
            ItemKind::General => (0.75, 0.75, 0.30),
            ItemKind::Scroll => (0.85, 0.80, 0.55),
            ItemKind::EmptyContainer => (0.55, 0.40, 0.25),
            ItemKind::Liquid => (0.30, 0.55, 0.90),
            ItemKind::Light | ItemKind::GlowStone => (1.0, 0.85, 0.35),
            ItemKind::Accessory => (0.85, 0.45, 0.85),
            ItemKind::TreasureChest => (0.85, 0.65, 0.25),
            _ => (0.6, 0.6, 0.65),
        }
    }
}

/// Equipment location. An item may fit several (dandan_spec「装備箇所」).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EquipSlot {
    Head,
    Neck,
    Body,
    Arm,
    Waist,
    Foot,
}

impl EquipSlot {
    /// All six slots in display order (index = `as_index`).
    pub const ALL: [EquipSlot; 6] = [
        EquipSlot::Head,
        EquipSlot::Neck,
        EquipSlot::Body,
        EquipSlot::Arm,
        EquipSlot::Waist,
        EquipSlot::Foot,
    ];

    pub fn as_index(self) -> usize {
        match self {
            EquipSlot::Head => 0,
            EquipSlot::Neck => 1,
            EquipSlot::Body => 2,
            EquipSlot::Arm => 3,
            EquipSlot::Waist => 4,
            EquipSlot::Foot => 5,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EquipSlot::Head => "頭",
            EquipSlot::Neck => "首",
            EquipSlot::Body => "体",
            EquipSlot::Arm => "腕",
            EquipSlot::Waist => "腰",
            EquipSlot::Foot => "足",
        }
    }
}

/// A stat change applied while equipped, or once when eaten. `duration_cycles`
/// of 0 means permanent (project spec「持続 0 = 永続」); equipment ignores the
/// duration (the effect lasts while worn).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatEffect {
    pub stat: StatKind,
    pub delta: i32,
    #[serde(default)]
    pub duration_cycles: u64,
}

/// An item definition (immutable; authored in `items.ron`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ItemDef {
    /// Unique within the project ("sword_iron", …).
    pub id: String,
    /// Display name (Japanese).
    pub name: String,
    pub kind: ItemKind,
    /// するどさ (weapon power; effective in plan6).
    #[serde(default)]
    pub sharpness: i32,
    /// かたさ (armour / how hard to eat).
    #[serde(default)]
    pub hardness: i32,
    /// おもさ (weight, ×100 g).
    #[serde(default)]
    pub weight: i32,
    #[serde(default)]
    pub temperature: i32,
    /// 栄養価 (HP change on eating; negative = damage).
    #[serde(default)]
    pub nutrition: i32,
    #[serde(default)]
    pub entropy_max: i32,
    #[serde(default)]
    pub anti_magic: i32,
    #[serde(default)]
    pub anti_appraisal: i32,
    #[serde(default)]
    pub anti_impact: i32,
    /// Important items can't be eaten or thrown.
    #[serde(default)]
    pub important: bool,
    #[serde(default)]
    pub throwability: i32,
    #[serde(default)]
    pub grip: i32,
    /// Storage capacity (containers only; >0).
    #[serde(default)]
    pub capacity: u32,
    /// Slots this item can occupy; empty = not equippable.
    #[serde(default)]
    pub equip_slots: Vec<EquipSlot>,
    /// Stat effects applied on equip / eat.
    #[serde(default)]
    pub effects: Vec<StatEffect>,
    /// glb path for the 3D floor display; empty = generic fallback shape.
    #[serde(default)]
    pub model: String,
}

impl ItemDef {
    pub fn is_equippable(&self) -> bool {
        !self.equip_slots.is_empty()
    }
}

/// A concrete item in the world / an inventory. `entropy` accumulates until the
/// transformation rules arrive (plan8); plan5 only carries it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ItemInstance {
    pub def_id: String,
    #[serde(default)]
    pub entropy: i32,
}

impl ItemInstance {
    pub fn new(def_id: impl Into<String>) -> Self {
        Self {
            def_id: def_id.into(),
            entropy: 0,
        }
    }
}

/// One item placed on a level (project format v3): an item id at a tile.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ItemPlacement {
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub floor: usize,
}

/// All item definitions, keyed by id (Bevy resource). Empty for a project with
/// no `items.ron` (v2 and earlier).
#[derive(Resource, Default)]
pub struct ItemCatalog {
    defs: HashMap<String, ItemDef>,
}

impl ItemCatalog {
    /// Build from a list, rejecting duplicate ids.
    pub fn from_defs(defs: Vec<ItemDef>, what: &str) -> Result<Self, String> {
        let mut map = HashMap::with_capacity(defs.len());
        for def in defs {
            if map.contains_key(&def.id) {
                return Err(format!("{what}: duplicate item id '{}'", def.id));
            }
            map.insert(def.id.clone(), def);
        }
        Ok(Self { defs: map })
    }

    pub fn get(&self, id: &str) -> Option<&ItemDef> {
        self.defs.get(id)
    }

    /// All definitions, in map order (callers needing determinism sort by id).
    pub fn iter(&self) -> impl Iterator<Item = &ItemDef> {
        self.defs.values()
    }
}

/// Addresses one inventory slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotRef {
    Hand(usize),
    Equip(EquipSlot),
    Pouch(usize),
    Backpack(usize),
}

/// A character's carried items: two hands, six equipment slots, and the
/// pouch/backpack (sized from `LimitsConfig`). Nested containers are out of
/// scope for plan5.
#[derive(Clone, Debug)]
pub struct Inventory {
    hands: [Option<ItemInstance>; 2],
    equipment: [Option<ItemInstance>; 6],
    pouch: Vec<Option<ItemInstance>>,
    backpack: Vec<Option<ItemInstance>>,
}

impl Inventory {
    pub fn new(pouch_size: usize, backpack_size: usize) -> Self {
        Self {
            hands: [None, None],
            equipment: Default::default(),
            pouch: vec![None; pouch_size],
            backpack: vec![None; backpack_size],
        }
    }

    fn cell(&self, r: SlotRef) -> Option<&Option<ItemInstance>> {
        match r {
            SlotRef::Hand(i) => self.hands.get(i),
            SlotRef::Equip(s) => self.equipment.get(s.as_index()),
            SlotRef::Pouch(i) => self.pouch.get(i),
            SlotRef::Backpack(i) => self.backpack.get(i),
        }
    }

    fn cell_mut(&mut self, r: SlotRef) -> Option<&mut Option<ItemInstance>> {
        match r {
            SlotRef::Hand(i) => self.hands.get_mut(i),
            SlotRef::Equip(s) => self.equipment.get_mut(s.as_index()),
            SlotRef::Pouch(i) => self.pouch.get_mut(i),
            SlotRef::Backpack(i) => self.backpack.get_mut(i),
        }
    }

    pub fn get(&self, r: SlotRef) -> Option<&ItemInstance> {
        self.cell(r).and_then(|c| c.as_ref())
    }

    /// Remove and return the item at `r` (if any).
    pub fn take(&mut self, r: SlotRef) -> Option<ItemInstance> {
        self.cell_mut(r).and_then(|c| c.take())
    }

    /// Put `item` into an empty slot `r`. Returns the item back if the slot is
    /// occupied or out of range.
    pub fn put(&mut self, r: SlotRef, item: ItemInstance) -> Result<(), ItemInstance> {
        match self.cell_mut(r) {
            Some(cell @ None) => {
                *cell = Some(item);
                Ok(())
            }
            _ => Err(item),
        }
    }

    /// First empty general-storage slot, hands → pouch → backpack (equipment is
    /// never a pickup target).
    pub fn first_free(&self) -> Option<SlotRef> {
        for i in 0..self.hands.len() {
            if self.hands[i].is_none() {
                return Some(SlotRef::Hand(i));
            }
        }
        for i in 0..self.pouch.len() {
            if self.pouch[i].is_none() {
                return Some(SlotRef::Pouch(i));
            }
        }
        for i in 0..self.backpack.len() {
            if self.backpack[i].is_none() {
                return Some(SlotRef::Backpack(i));
            }
        }
        None
    }

    /// Store `item` in the first free general slot. Returns where it went, or the
    /// item back when full.
    pub fn pickup(&mut self, item: ItemInstance) -> Result<SlotRef, ItemInstance> {
        match self.first_free() {
            Some(r) => {
                let _ = self.put(r, item);
                Ok(r)
            }
            None => Err(item),
        }
    }

    /// Iterate every held item with its slot.
    pub fn iter(&self) -> impl Iterator<Item = (SlotRef, &ItemInstance)> {
        let hands = self
            .hands
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.as_ref().map(|it| (SlotRef::Hand(i), it)));
        let equip = EquipSlot::ALL
            .iter()
            .filter_map(|s| self.equipment[s.as_index()].as_ref().map(|it| (SlotRef::Equip(*s), it)));
        let pouch = self
            .pouch
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.as_ref().map(|it| (SlotRef::Pouch(i), it)));
        let backpack = self
            .backpack
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.as_ref().map(|it| (SlotRef::Backpack(i), it)));
        hands.chain(equip).chain(pouch).chain(backpack)
    }

    /// Total carried weight (×100 g), summed over every slot.
    pub fn total_weight(&self, catalog: &ItemCatalog) -> i32 {
        self.iter()
            .filter_map(|(_, it)| catalog.get(&it.def_id))
            .map(|d| d.weight)
            .sum()
    }

    /// Passive `(stat, delta)` contributions from the six equipment slots only.
    pub fn equipment_effects(&self, catalog: &ItemCatalog) -> Vec<(StatKind, i32)> {
        let mut out = Vec::new();
        for s in EquipSlot::ALL {
            if let Some(it) = &self.equipment[s.as_index()]
                && let Some(def) = catalog.get(&it.def_id)
            {
                for e in &def.effects {
                    out.push((e.stat, e.delta));
                }
            }
        }
        out
    }

    /// Equip the item at `from` (a hand/pouch/backpack slot) into its first
    /// matching equipment slot, swapping any current occupant back to `from`.
    pub fn equip(&mut self, from: SlotRef, catalog: &ItemCatalog) -> Result<(), String> {
        if matches!(from, SlotRef::Equip(_)) {
            return Err("すでに装備している".into());
        }
        let Some(item) = self.get(from).cloned() else {
            return Err("スロットが空だ".into());
        };
        let def = catalog
            .get(&item.def_id)
            .ok_or_else(|| format!("未知のアイテム '{}'", item.def_id))?;
        let Some(&slot) = def.equip_slots.first() else {
            return Err(format!("{}は装備できない", def.name));
        };
        let target = SlotRef::Equip(slot);
        let item = self.take(from).expect("checked present");
        let previous = self.take(target);
        let _ = self.put(target, item);
        if let Some(old) = previous {
            // `from` is now free — return the displaced item there.
            let _ = self.put(from, old);
        }
        Ok(())
    }

    /// Unequip `slot` into the first free general slot. Fails if nothing is free.
    pub fn unequip(&mut self, slot: EquipSlot) -> Result<(), String> {
        let src = SlotRef::Equip(slot);
        if self.get(src).is_none() {
            return Err("装備していない".into());
        }
        let Some(dest) = self.first_free() else {
            return Err("持ちきれない(空きがない)".into());
        };
        let item = self.take(src).expect("checked present");
        let _ = self.put(dest, item);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> ItemCatalog {
        let sword = ItemDef {
            id: "sword".into(),
            name: "つるぎ".into(),
            kind: ItemKind::General,
            weight: 30,
            equip_slots: vec![EquipSlot::Arm],
            effects: vec![StatEffect {
                stat: StatKind::Attack,
                delta: 5,
                duration_cycles: 0,
            }],
            ..blank("sword")
        };
        let helm = ItemDef {
            id: "helm".into(),
            name: "かぶと".into(),
            weight: 20,
            equip_slots: vec![EquipSlot::Head],
            ..blank("helm")
        };
        ItemCatalog::from_defs(vec![sword, helm], "test").unwrap()
    }

    fn blank(id: &str) -> ItemDef {
        ItemDef {
            id: id.into(),
            name: id.into(),
            kind: ItemKind::General,
            sharpness: 0,
            hardness: 0,
            weight: 0,
            temperature: 0,
            nutrition: 0,
            entropy_max: 0,
            anti_magic: 0,
            anti_appraisal: 0,
            anti_impact: 0,
            important: false,
            throwability: 0,
            grip: 0,
            capacity: 0,
            equip_slots: vec![],
            effects: vec![],
            model: String::new(),
        }
    }

    #[test]
    fn pickup_fills_hands_then_pouch() {
        let mut inv = Inventory::new(1, 0);
        assert_eq!(inv.pickup(ItemInstance::new("a")), Ok(SlotRef::Hand(0)));
        assert_eq!(inv.pickup(ItemInstance::new("b")), Ok(SlotRef::Hand(1)));
        assert_eq!(inv.pickup(ItemInstance::new("c")), Ok(SlotRef::Pouch(0)));
        // Full now.
        assert!(inv.pickup(ItemInstance::new("d")).is_err());
    }

    #[test]
    fn weight_sums_all_slots() {
        let cat = catalog();
        let mut inv = Inventory::new(3, 3);
        inv.pickup(ItemInstance::new("sword")).unwrap();
        inv.pickup(ItemInstance::new("helm")).unwrap();
        assert_eq!(inv.total_weight(&cat), 50);
    }

    #[test]
    fn equip_then_unequip_round_trips() {
        let cat = catalog();
        let mut inv = Inventory::new(3, 3);
        inv.pickup(ItemInstance::new("sword")).unwrap();
        inv.equip(SlotRef::Hand(0), &cat).unwrap();
        assert!(inv.get(SlotRef::Equip(EquipSlot::Arm)).is_some());
        assert_eq!(inv.equipment_effects(&cat), vec![(StatKind::Attack, 5)]);
        inv.unequip(EquipSlot::Arm).unwrap();
        assert!(inv.get(SlotRef::Equip(EquipSlot::Arm)).is_none());
        assert!(inv.equipment_effects(&cat).is_empty());
    }
}
