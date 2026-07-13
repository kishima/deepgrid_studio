//! Data screen (plan5): the inventory / equipment / status overlay, reached with
//! `Tab` or `I` (the modernised form of the original's right-click screen —
//! project.md「UIの方針」). Built with **bevy_ui** so it appears in screenshots.
//!
//! The world keeps simulating while it's open; only movement/pickup pause. UI is
//! spawned when the screen opens and torn down when it closes; a per-frame
//! refresh keeps its text in sync, and a click handler runs the actions
//! (equip / unequip / eat / place / ZZZ).

use bevy::prelude::*;

use crate::character::{Party, StatKind};
use crate::clock::CycleTick;
use crate::floor_items::PlaceRequest;
use crate::game_state::{DataScreen, SelectedMember};
use crate::hud::MessageLog;
use crate::item::{EquipSlot, ItemCatalog, SlotRef};

const FONT_REGULAR: &str = "fonts/PixelMplus12-Regular.ttf";
const FONT_BOLD: &str = "fonts/PixelMplus12-Bold.ttf";

const SCREEN_BG: Color = Color::srgb(0.06, 0.06, 0.09);
const PANEL_BG: Color = Color::srgb(0.11, 0.11, 0.15);
const SLOT_BG: Color = Color::srgb(0.16, 0.16, 0.21);
const SLOT_SEL: Color = Color::srgb(0.30, 0.42, 0.28);
const BTN_BG: Color = Color::srgb(0.22, 0.24, 0.30);
const TAB_BG: Color = Color::srgb(0.16, 0.16, 0.21);
const TAB_SEL: Color = Color::srgb(0.28, 0.34, 0.46);

/// Which inventory slot the player last clicked (target of the action buttons).
#[derive(Resource, Default)]
pub struct SelectedSlot {
    pub slot: Option<SlotRef>,
}

/// ZZZ rest state: while open + resting, the party heals slowly on a cycle timer.
#[derive(Resource, Default)]
pub struct Resting {
    pub active: bool,
}

/// The spawned data-screen root, so it can be despawned on close.
#[derive(Resource, Default)]
pub struct DataScreenRoot(Option<Entity>);

/// One action button's meaning.
#[derive(Clone, Copy, PartialEq)]
enum ActionKind {
    Equip,
    Unequip,
    Eat,
    Place,
    Zzz,
}

#[derive(Component)]
pub struct MemberTab(usize);
#[derive(Component)]
pub struct SlotButton(SlotRef);
/// The value text of a slot. `prefix` is the row label ("手L", "頭", …) for the
/// equipment rows, or empty for the bare pouch/backpack cells.
#[derive(Component)]
pub struct SlotLabel {
    slot: SlotRef,
    prefix: &'static str,
}
#[derive(Component)]
pub struct ActionBtn(ActionKind);
#[derive(Component)]
pub struct DetailText;
#[derive(Component)]
pub struct StatusText;
#[derive(Component)]
pub struct WeightText;

/// Rest cycle cost: heal 1 HP/MP every this many cycles while ZZZ-resting.
const REST_CYCLES: u64 = 10;

/// Register the data-screen resources.
pub fn init(app: &mut App) {
    app.init_resource::<SelectedSlot>()
        .init_resource::<Resting>()
        .init_resource::<DataScreenRoot>()
        .add_event::<PlaceRequest>();
}

/// Open/close the overlay when `DataScreen.open` flips.
#[allow(clippy::too_many_arguments)]
pub fn toggle_data_screen(
    mut commands: Commands,
    screen: Res<DataScreen>,
    mut root: ResMut<DataScreenRoot>,
    mut resting: ResMut<Resting>,
    mut selected_slot: ResMut<SelectedSlot>,
    asset_server: Res<AssetServer>,
    party: Res<Party>,
    limits: Res<crate::config::LimitsConfig>,
) {
    if !screen.is_changed() {
        return;
    }
    match (screen.open, root.0) {
        (true, None) => {
            let regular: Handle<Font> = asset_server.load(FONT_REGULAR);
            let bold: Handle<Font> = asset_server.load(FONT_BOLD);
            let e = build_screen(&mut commands, &regular, &bold, &party, &limits);
            root.0 = Some(e);
        }
        (false, Some(e)) => {
            commands.entity(e).despawn_recursive();
            root.0 = None;
            resting.active = false;
            selected_slot.slot = None;
        }
        _ => {}
    }
}

fn text(font: &Handle<Font>, size: f32, color: Color, s: impl Into<String>) -> impl Bundle {
    (
        Text::new(s.into()),
        TextFont {
            font: font.clone(),
            font_size: size,
            ..default()
        },
        TextColor(color),
    )
}

/// Build the full overlay; returns the root entity.
fn build_screen(
    commands: &mut Commands,
    regular: &Handle<Font>,
    bold: &Handle<Font>,
    party: &Party,
    limits: &crate::config::LimitsConfig,
) -> Entity {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(10.0)),
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(SCREEN_BG),
        ))
        .with_children(|root| {
            // Title + member tabs.
            root.spawn(text(bold, 20.0, Color::srgb(0.9, 0.9, 0.8), "データ画面  (Tab/I で戻る)"));
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|tabs| {
                for (i, m) in party.members.iter().enumerate() {
                    tabs.spawn((
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(TAB_BG),
                        MemberTab(i),
                    ))
                    .with_children(|b| {
                        b.spawn(text(bold, 15.0, Color::WHITE, format!("P{}: {}", i + 1, m.character.first_name)));
                    });
                }
            });

            // Body: three columns.
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(10.0),
                flex_grow: 1.0,
                ..default()
            })
            .with_children(|body| {
                // Left: equipment + hands.
                body.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        width: Val::Px(230.0),
                        row_gap: Val::Px(4.0),
                        padding: UiRect::all(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(PANEL_BG),
                ))
                .with_children(|col| {
                    col.spawn(text(bold, 15.0, Color::srgb(0.8, 0.85, 0.9), "装備 / 手"));
                    slot_row(col, regular, SlotRef::Hand(0), "手L");
                    slot_row(col, regular, SlotRef::Hand(1), "手R");
                    for s in EquipSlot::ALL {
                        slot_row(col, regular, SlotRef::Equip(s), s.label());
                    }
                });

                // Middle: pouch + backpack.
                body.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        flex_grow: 1.0,
                        row_gap: Val::Px(6.0),
                        padding: UiRect::all(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(PANEL_BG),
                ))
                .with_children(|col| {
                    col.spawn(text(bold, 15.0, Color::srgb(0.8, 0.85, 0.9), "ポーチ"));
                    col.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(4.0),
                        flex_wrap: FlexWrap::Wrap,
                        ..default()
                    })
                    .with_children(|row| {
                        for i in 0..limits.pouch_size {
                            slot_cell(row, regular, SlotRef::Pouch(i));
                        }
                    });
                    col.spawn(text(bold, 15.0, Color::srgb(0.8, 0.85, 0.9), "リュック"));
                    col.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(4.0),
                        row_gap: Val::Px(4.0),
                        flex_wrap: FlexWrap::Wrap,
                        width: Val::Px(420.0),
                        ..default()
                    })
                    .with_children(|grid| {
                        for i in 0..limits.backpack_size {
                            slot_cell(grid, regular, SlotRef::Backpack(i));
                        }
                    });
                });

                // Right: detail + actions.
                body.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        width: Val::Px(240.0),
                        row_gap: Val::Px(6.0),
                        padding: UiRect::all(Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(PANEL_BG),
                ))
                .with_children(|col| {
                    col.spawn(text(bold, 15.0, Color::srgb(0.8, 0.85, 0.9), "詳細"));
                    col.spawn((text(regular, 13.0, Color::srgb(0.85, 0.85, 0.8), "スロットを選択"), DetailText));
                    col.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(4.0),
                        flex_wrap: FlexWrap::Wrap,
                        row_gap: Val::Px(4.0),
                        ..default()
                    })
                    .with_children(|row| {
                        action_btn(row, bold, ActionKind::Equip, "装備");
                        action_btn(row, bold, ActionKind::Unequip, "はずす");
                        action_btn(row, bold, ActionKind::Eat, "食べる");
                        action_btn(row, bold, ActionKind::Place, "置く");
                        action_btn(row, bold, ActionKind::Zzz, "ZZZ 休息");
                    });
                });
            });

            // Footer: status + weight.
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
            ))
            .with_children(|col| {
                col.spawn((text(regular, 13.0, Color::srgb(0.85, 0.85, 0.8), ""), StatusText));
                col.spawn((text(regular, 13.0, Color::srgb(0.85, 0.85, 0.8), ""), WeightText));
            });
        })
        .id()
}

/// A labelled equipment/hand row (label + value in one clickable button).
fn slot_row(col: &mut ChildBuilder, font: &Handle<Font>, slot: SlotRef, label: &'static str) {
    col.spawn((
        Button,
        Node {
            padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
            width: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(SLOT_BG),
        SlotButton(slot),
    ))
    .with_children(|b| {
        b.spawn((
            Text::new(format!("{label}: ―")),
            TextFont { font: font.clone(), font_size: 13.0, ..default() },
            TextColor(Color::srgb(0.9, 0.9, 0.85)),
            SlotLabel { slot, prefix: label },
        ));
    });
}

/// A small square inventory cell (pouch / backpack).
fn slot_cell(row: &mut ChildBuilder, font: &Handle<Font>, slot: SlotRef) {
    row.spawn((
        Button,
        Node {
            width: Val::Px(64.0),
            height: Val::Px(28.0),
            padding: UiRect::all(Val::Px(2.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(SLOT_BG),
        SlotButton(slot),
    ))
    .with_children(|b| {
        b.spawn((
            Text::new("・"),
            TextFont { font: font.clone(), font_size: 11.0, ..default() },
            TextColor(Color::srgb(0.9, 0.9, 0.85)),
            SlotLabel { slot, prefix: "" },
        ));
    });
}

fn action_btn(row: &mut ChildBuilder, font: &Handle<Font>, kind: ActionKind, label: &str) {
    row.spawn((
        Button,
        Node {
            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(BTN_BG),
        ActionBtn(kind),
    ))
    .with_children(|b| {
        b.spawn(text(font, 13.0, Color::WHITE, label));
    });
}

/// Per-frame refresh of the overlay's text + selection highlight.
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub fn refresh_data_screen(
    screen: Res<DataScreen>,
    party: Res<Party>,
    catalog: Res<ItemCatalog>,
    selected: Res<SelectedMember>,
    selected_slot: Res<SelectedSlot>,
    resting: Res<Resting>,
    rules: Res<crate::rules::RulesConfig>,
    mut sets: ParamSet<(
        Query<(&SlotButton, &mut BackgroundColor)>,
        Query<(&SlotLabel, &mut Text)>,
        Query<&mut Text, With<DetailText>>,
        Query<&mut Text, With<StatusText>>,
        Query<&mut Text, With<WeightText>>,
        Query<(&MemberTab, &mut BackgroundColor)>,
    )>,
) {
    if !screen.open {
        return;
    }
    let Some(member) = party.members.get(selected.index) else {
        return;
    };

    // Slot highlight.
    for (btn, mut bg) in &mut sets.p0() {
        bg.0 = if selected_slot.slot == Some(btn.0) { SLOT_SEL } else { SLOT_BG };
    }
    // Slot labels ("手L: つるぎ" for equip rows, bare name for pouch cells).
    for (label, mut t) in &mut sets.p1() {
        let name = member
            .inventory
            .get(label.slot)
            .and_then(|it| catalog.get(&it.def_id))
            .map(|d| d.name.clone());
        **t = match (label.prefix.is_empty(), name) {
            (true, Some(n)) => n,
            (true, None) => "・".to_string(),
            (false, Some(n)) => format!("{}: {}", label.prefix, n),
            (false, None) => format!("{}: ―", label.prefix),
        };
    }
    // Detail.
    {
        let mut p = sets.p2();
        if let Ok(mut t) = p.get_single_mut() {
            **t = selected_slot
                .slot
                .and_then(|s| member.inventory.get(s))
                .and_then(|it| catalog.get(&it.def_id))
                .map(|d| {
                    let effects = if d.effects.is_empty() {
                        String::new()
                    } else {
                        let parts: Vec<String> = d
                            .effects
                            .iter()
                            .map(|e| format!("{:?}{:+}", e.stat, e.delta))
                            .collect();
                        format!("\n効果: {}", parts.join(", "))
                    };
                    format!(
                        "{}  [{}]\n重さ {} / するどさ {} / かたさ {}\n栄養 {}{}{}",
                        d.name,
                        d.kind.label(),
                        d.weight,
                        d.sharpness,
                        d.hardness,
                        d.nutrition,
                        if d.important { "  ※だいじなもの" } else { "" },
                        effects,
                    )
                })
                .unwrap_or_else(|| "スロットを選択".to_string());
        }
    }
    // Status.
    let eff = member.effective_stats(&catalog);
    {
        let mut p = sets.p3();
        if let Ok(mut t) = p.get_single_mut() {
            let satiety = if rules.hunger.enabled {
                format!("  満腹度 {}/{}", member.state.satiety, rules.hunger.satiety_max)
            } else {
                String::new()
            };
            **t = format!(
                "{}  総合Lv {}  |  HP {}/{}  MP {}/{}  集 {}/{}{satiety}\n\
                 攻 {} 防 {} 速 {} 運搬 {} 肺 {} 耐熱 {} 耐毒 {}  |  経歴: {}",
                member.character.first_name,
                eff.overall_level(),
                member.state.hp,
                eff.get(StatKind::MaxHp),
                member.state.mp,
                eff.get(StatKind::MaxMp),
                member.state.concentration,
                member.character.stats.concentration,
                eff.get(StatKind::Attack),
                eff.get(StatKind::Defense),
                eff.get(StatKind::Agility),
                eff.get(StatKind::Carrying),
                eff.get(StatKind::LungCapacity),
                eff.get(StatKind::HeatResist),
                eff.get(StatKind::PoisonResist),
                member.character.background.replace('\n', " "),
            );
        }
    }
    // Weight.
    let weight = member.inventory.total_weight(&catalog);
    let carrying = eff.get(StatKind::Carrying);
    {
        let mut p = sets.p4();
        if let Ok(mut t) = p.get_single_mut() {
            **t = format!(
                "総重量 {}00g / 運搬力 {}00g{}{}",
                weight,
                carrying,
                if weight > carrying { "  ※重すぎる!" } else { "" },
                if resting.active { "   [休息中 ZZZ]" } else { "" },
            );
        }
    }
    // Member tab highlight.
    for (tab, mut bg) in &mut sets.p5() {
        bg.0 = if tab.0 == selected.index { TAB_SEL } else { TAB_BG };
    }
}

/// Handle clicks on tabs / slots / action buttons.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn data_screen_interactions(
    screen: Res<DataScreen>,
    mut party: ResMut<Party>,
    catalog: Res<ItemCatalog>,
    mut selected: ResMut<SelectedMember>,
    mut selected_slot: ResMut<SelectedSlot>,
    mut resting: ResMut<Resting>,
    mut log: ResMut<MessageLog>,
    enemy_near: Res<crate::monster::EnemyNear>,
    rules: Res<crate::rules::RulesConfig>,
    mut place: EventWriter<PlaceRequest>,
    tabs: Query<(&Interaction, &MemberTab), Changed<Interaction>>,
    slots: Query<(&Interaction, &SlotButton), Changed<Interaction>>,
    actions: Query<(&Interaction, &ActionBtn), Changed<Interaction>>,
) {
    if !screen.open {
        return;
    }
    for (interaction, tab) in &tabs {
        if *interaction == Interaction::Pressed {
            selected.index = tab.0;
            selected_slot.slot = None;
        }
    }
    for (interaction, slot) in &slots {
        if *interaction == Interaction::Pressed {
            selected_slot.slot = Some(slot.0);
        }
    }
    let Some(member) = party.members.get_mut(selected.index) else {
        return;
    };
    for (interaction, action) in &actions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if action.0 == ActionKind::Zzz {
            if !resting.active && enemy_near.0 {
                log.push("モンスターが近くにいる!");
                continue;
            }
            resting.active = !resting.active;
            log.push(if resting.active { "休息を始めた" } else { "休息をやめた" });
            continue;
        }
        let Some(slot) = selected_slot.slot else {
            log.push("スロットを選択してください");
            continue;
        };
        match action.0 {
            ActionKind::Equip => match member.inventory.equip(slot, &catalog) {
                Ok(()) => {
                    log.push("装備した");
                    selected_slot.slot = None;
                }
                Err(e) => log.push(e),
            },
            ActionKind::Unequip => {
                if let SlotRef::Equip(s) = slot {
                    match member.inventory.unequip(s) {
                        Ok(()) => log.push("はずした"),
                        Err(e) => log.push(e),
                    }
                } else {
                    log.push("装備スロットではない");
                }
            }
            ActionKind::Eat => {
                let def = member
                    .inventory
                    .get(slot)
                    .and_then(|it| catalog.get(&it.def_id))
                    .cloned();
                match def {
                    Some(def) => match member.eat(&def, &catalog, &rules.hunger) {
                        Ok(msg) => {
                            member.inventory.take(slot);
                            selected_slot.slot = None;
                            log.push(msg);
                        }
                        Err(e) => log.push(e),
                    },
                    None => log.push("スロットが空だ"),
                }
            }
            ActionKind::Place => {
                if member.inventory.get(slot).is_some() {
                    place.send(PlaceRequest { slot });
                    selected_slot.slot = None;
                } else {
                    log.push("スロットが空だ");
                }
            }
            ActionKind::Zzz => {}
        }
    }
}

/// ZZZ rest: while the data screen is open and resting, heal 1 HP/MP every
/// [`REST_CYCLES`] cycles, up to each member's maximum.
#[allow(clippy::too_many_arguments)]
pub fn rest_tick(
    mut accum: Local<u64>,
    mut ticks: EventReader<CycleTick>,
    screen: Res<DataScreen>,
    mut resting: ResMut<Resting>,
    enemy_near: Res<crate::monster::EnemyNear>,
    rules: Res<crate::rules::RulesConfig>,
    mut log: ResMut<MessageLog>,
    catalog: Res<ItemCatalog>,
    mut party: ResMut<Party>,
) {
    let cycles = ticks.read().count() as u64;
    // A monster coming into view interrupts the rest (plan6).
    if resting.active && enemy_near.0 {
        resting.active = false;
        log.push("モンスターが現れて 休息を中断した!");
    }
    if !screen.open || !resting.active {
        *accum = 0;
        return;
    }
    *accum += cycles;
    while *accum >= REST_CYCLES {
        *accum -= REST_CYCLES;
        for member in &mut party.members {
            if member.state.down {
                continue;
            }
            // Starving members don't recover from rest (plan6.5).
            if rules.hunger.enabled && member.state.satiety == 0 {
                continue;
            }
            let eff = member.effective_stats(&catalog);
            member.state.hp = (member.state.hp + 1).min(eff.get(StatKind::MaxHp));
            member.state.mp = (member.state.mp + 1).min(eff.get(StatKind::MaxMp));
        }
    }
}
