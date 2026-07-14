//! Data screen (plan5 + plan7): the inventory / equipment / status overlay,
//! reached with `Tab` / `I`, plus the **magic tab** (`M` or the 魔法 icon).
//!
//! Two views share one overlay: the item view (equipment / pouch / backpack /
//! detail + actions) and the magic view (learned-spell list + cast / liquefy).
//! Both are built once; a view toggle only flips which body is displayed, so no
//! rebuild churn on tab switches. The world keeps simulating while it's open;
//! only movement/pickup pause.

use bevy::prelude::*;

use crate::character::{Party, StatKind};
use crate::clock::CycleTick;
use crate::floor_items::PlaceRequest;
use crate::game_state::{DataScreen, DataView, SelectedMember};
use crate::hud::MessageLog;
use crate::item::{EquipSlot, ItemCatalog, ItemInstance, SlotRef};
use crate::magic::{CastMagic, CastTarget, MagicCatalog, SelectedMagic};

const FONT_REGULAR: &str = "fonts/PixelMplus12-Regular.ttf";
const FONT_BOLD: &str = "fonts/PixelMplus12-Bold.ttf";

const SCREEN_BG: Color = Color::srgb(0.06, 0.06, 0.09);
const PANEL_BG: Color = Color::srgb(0.11, 0.11, 0.15);
const SLOT_BG: Color = Color::srgb(0.16, 0.16, 0.21);
const SLOT_SEL: Color = Color::srgb(0.30, 0.42, 0.28);
const SLOT_DISABLED: Color = Color::srgb(0.10, 0.10, 0.12);
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

/// One item-view action button's meaning.
#[derive(Clone, Copy, PartialEq)]
enum ActionKind {
    Equip,
    Unequip,
    Eat,
    Place,
    Read,
    Drink,
    Zzz,
}

/// One magic-view action button's meaning.
#[derive(Clone, Copy, PartialEq)]
enum MagicAct {
    Cast,
    Liquefy,
}

#[derive(Component)]
pub struct MemberTab(usize);
/// View toggle tab: `true` = magic view, `false` = item view.
#[derive(Component)]
pub struct ViewTab(bool);
/// The item-view body container (shown when `!DataView.magic`).
#[derive(Component)]
pub struct ItemBody;
/// The magic-view body container (shown when `DataView.magic`).
#[derive(Component)]
pub struct MagicBody;
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
/// A magic-list row button, carrying the magic id it casts.
#[derive(Component)]
pub struct MagicRow(String);
/// The text inside a magic row (symbol + name + MP), carrying the magic id.
#[derive(Component)]
pub struct MagicRowText(String);
#[derive(Component)]
pub struct MagicDetailText;
#[derive(Component)]
pub struct MagicActBtn(MagicAct);
/// An ally-target selector button in the magic view (a party slot).
#[derive(Component)]
pub struct MagicTargetBtn(usize);

/// Rest cycle cost: heal 1 HP/MP every this many cycles while ZZZ-resting.
const REST_CYCLES: u64 = 10;

/// Register the data-screen resources.
pub fn init(app: &mut App) {
    app.init_resource::<SelectedSlot>()
        .init_resource::<Resting>()
        .init_resource::<DataScreenRoot>()
        .add_event::<PlaceRequest>();
}

/// Open/close the overlay when `DataScreen.open` flips (or the world's magic
/// catalog is needed for a fresh build). The magic-list rows are built here from
/// the catalog, hidden per member at refresh.
#[allow(clippy::too_many_arguments)]
pub fn toggle_data_screen(
    mut commands: Commands,
    screen: Res<DataScreen>,
    mut root: ResMut<DataScreenRoot>,
    mut resting: ResMut<Resting>,
    mut selected_slot: ResMut<SelectedSlot>,
    asset_server: Res<AssetServer>,
    party: Res<Party>,
    magics: Res<MagicCatalog>,
    limits: Res<crate::config::LimitsConfig>,
) {
    if !screen.is_changed() {
        return;
    }
    match (screen.open, root.0) {
        (true, None) => {
            let regular: Handle<Font> = asset_server.load(FONT_REGULAR);
            let bold: Handle<Font> = asset_server.load(FONT_BOLD);
            let e = build_screen(&mut commands, &regular, &bold, &party, &magics, &limits);
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
    magics: &MagicCatalog,
    limits: &crate::config::LimitsConfig,
) -> Entity {
    // Deterministic magic order for the list.
    let mut magic_ids: Vec<String> = magics.iter().map(|d| d.id.clone()).collect();
    magic_ids.sort();

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
            root.spawn(text(bold, 20.0, Color::srgb(0.9, 0.9, 0.8), "データ画面  (Tab/I で戻る)"));

            // View tabs (もちもの / まほう).
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|tabs| {
                for (magic, label) in [(false, "もちもの"), (true, "まほう")] {
                    tabs.spawn((
                        Button,
                        Node { padding: UiRect::axes(Val::Px(12.0), Val::Px(4.0)), ..default() },
                        BackgroundColor(TAB_BG),
                        ViewTab(magic),
                    ))
                    .with_children(|b| {
                        b.spawn(text(bold, 15.0, Color::WHITE, label));
                    });
                }
            });

            // Member tabs.
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|tabs| {
                for (i, m) in party.members.iter().enumerate() {
                    tabs.spawn((
                        Button,
                        Node { padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)), ..default() },
                        BackgroundColor(TAB_BG),
                        MemberTab(i),
                    ))
                    .with_children(|b| {
                        b.spawn(text(bold, 15.0, Color::WHITE, format!("P{}: {}", i + 1, m.character.first_name)));
                    });
                }
            });

            // Item-view body (three columns).
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(10.0),
                    flex_grow: 1.0,
                    ..default()
                },
                ItemBody,
            ))
            .with_children(|body| {
                build_item_body(body, regular, bold, limits);
            });

            // Magic-view body (hidden until the まほう tab is selected).
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(10.0),
                    flex_grow: 1.0,
                    display: Display::None,
                    ..default()
                },
                MagicBody,
            ))
            .with_children(|body| {
                build_magic_body(body, regular, bold, party, &magic_ids);
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

/// Build the item-view columns (equipment/hands, pouch/backpack, detail+actions).
fn build_item_body(
    body: &mut ChildBuilder,
    regular: &Handle<Font>,
    bold: &Handle<Font>,
    limits: &crate::config::LimitsConfig,
) {
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
            action_btn(row, bold, ActionKind::Read, "見る");
            action_btn(row, bold, ActionKind::Drink, "飲む");
            action_btn(row, bold, ActionKind::Place, "置く");
            action_btn(row, bold, ActionKind::Zzz, "ZZZ 休息");
        });
    });
}

/// Build the magic-view columns (spell list, detail + cast/liquefy + targets).
fn build_magic_body(
    body: &mut ChildBuilder,
    regular: &Handle<Font>,
    bold: &Handle<Font>,
    party: &Party,
    magic_ids: &[String],
) {
    // Left: the learned-spell list.
    body.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            width: Val::Px(320.0),
            row_gap: Val::Px(4.0),
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
    ))
    .with_children(|col| {
        col.spawn(text(bold, 15.0, Color::srgb(0.8, 0.85, 0.9), "習得した魔法"));
        for id in magic_ids {
            col.spawn((
                Button,
                Node {
                    padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                    width: Val::Percent(100.0),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(SLOT_BG),
                MagicRow(id.clone()),
            ))
            .with_children(|b| {
                b.spawn((
                    text(regular, 13.0, Color::srgb(0.9, 0.9, 0.85), id.clone()),
                    MagicRowText(id.clone()),
                ));
            });
        }
    });

    // Right: detail + cast / liquefy + target selector.
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
        col.spawn(text(bold, 15.0, Color::srgb(0.8, 0.85, 0.9), "詳細"));
        col.spawn((text(regular, 13.0, Color::srgb(0.85, 0.85, 0.8), "魔法を選択"), MagicDetailText));
        col.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(4.0),
            flex_wrap: FlexWrap::Wrap,
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|row| {
            magic_act_btn(row, bold, MagicAct::Cast, "唱える");
            magic_act_btn(row, bold, MagicAct::Liquefy, "液体化");
        });
        col.spawn(text(regular, 12.0, Color::srgb(0.7, 0.72, 0.78), "対象(味方):"));
        col.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(4.0),
            flex_wrap: FlexWrap::Wrap,
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|row| {
            for (i, m) in party.members.iter().enumerate() {
                row.spawn((
                    Button,
                    Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)), ..default() },
                    BackgroundColor(BTN_BG),
                    MagicTargetBtn(i),
                ))
                .with_children(|b| {
                    b.spawn(text(regular, 12.0, Color::WHITE, m.character.first_name.clone()));
                });
            }
        });
    });
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
        Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)), ..default() },
        BackgroundColor(BTN_BG),
        ActionBtn(kind),
    ))
    .with_children(|b| {
        b.spawn(text(font, 13.0, Color::WHITE, label));
    });
}

fn magic_act_btn(row: &mut ChildBuilder, font: &Handle<Font>, act: MagicAct, label: &str) {
    row.spawn((
        Button,
        Node { padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)), ..default() },
        BackgroundColor(BTN_BG),
        MagicActBtn(act),
    ))
    .with_children(|b| {
        b.spawn(text(font, 13.0, Color::WHITE, label));
    });
}

/// Display name of an inventory instance: a potion shows "〜のビン".
fn display_name(inst: &ItemInstance, catalog: &ItemCatalog, magics: &MagicCatalog) -> String {
    if let Some(mid) = &inst.potion_of {
        let mname = magics.get(mid).map(|d| d.name.clone()).unwrap_or_else(|| mid.clone());
        return format!("{mname}のビン");
    }
    catalog
        .get(&inst.def_id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| inst.def_id.clone())
}

/// Per-frame refresh of the item view's text + selection highlight.
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub fn refresh_data_screen(
    screen: Res<DataScreen>,
    party: Res<Party>,
    catalog: Res<ItemCatalog>,
    magics: Res<MagicCatalog>,
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
        let name = member.inventory.get(label.slot).map(|it| display_name(it, &catalog, &magics));
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
                .and_then(|s| member.inventory.get(s).map(|it| (s, it)))
                .and_then(|(_, it)| catalog.get(&it.def_id).map(|d| (it, d)))
                .map(|(it, d)| {
                    let potion = it
                        .potion_of
                        .as_ref()
                        .map(|mid| {
                            let mn = magics.get(mid).map(|m| m.name.clone()).unwrap_or_else(|| mid.clone());
                            format!("\n秘薬: 『{mn}』(飲む/投げつける)")
                        })
                        .unwrap_or_default();
                    let effects = if d.effects.is_empty() {
                        String::new()
                    } else {
                        let parts: Vec<String> = d
                            .effects
                            .iter()
                            .map(|e| format!("{}{:+}", e.stat.label(), e.delta))
                            .collect();
                        format!("\n効果: {}", parts.join(", "))
                    };
                    format!(
                        "{}  [{}]\n重さ {} / するどさ {} / かたさ {}\n栄養 {}{}{}{}",
                        display_name(it, &catalog, &magics),
                        d.kind.label(),
                        d.weight,
                        d.sharpness,
                        d.hardness,
                        d.nutrition,
                        if d.important { "  ※だいじなもの" } else { "" },
                        potion,
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

/// Per-frame refresh of the magic view: body visibility, spell rows (shown only
/// for known spells, greyed when knowledge is too low), detail, and target tabs.
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub fn refresh_magic_screen(
    screen: Res<DataScreen>,
    view: Res<DataView>,
    party: Res<Party>,
    catalog: Res<ItemCatalog>,
    magics: Res<MagicCatalog>,
    selected: Res<SelectedMember>,
    selected_magic: Res<SelectedMagic>,
    mut sets: ParamSet<(
        Query<(&mut Node, Has<MagicBody>), Or<(With<ItemBody>, With<MagicBody>)>>,
        Query<(&ViewTab, &mut BackgroundColor)>,
        Query<(&MagicRow, &mut Node, &mut BackgroundColor)>,
        Query<(&MagicRowText, &mut Text, &mut TextColor)>,
        Query<&mut Text, With<MagicDetailText>>,
        Query<(&MagicTargetBtn, &mut BackgroundColor)>,
    )>,
) {
    if !screen.open {
        return;
    }
    // Body visibility.
    for (mut node, is_magic) in &mut sets.p0() {
        node.display = if is_magic == view.magic { Display::Flex } else { Display::None };
    }
    // View-tab highlight.
    for (tab, mut bg) in &mut sets.p1() {
        bg.0 = if tab.0 == view.magic { TAB_SEL } else { TAB_BG };
    }
    let Some(member) = party.members.get(selected.index) else {
        return;
    };
    let knowledge = member.effective_stats(&catalog).get(StatKind::MagicKnowledge);
    let known = |id: &str| member.state.learned.iter().any(|m| m == id);

    // Row visibility + highlight.
    for (row, mut node, mut bg) in &mut sets.p2() {
        let show = known(&row.0);
        node.display = if show { Display::Flex } else { Display::None };
        let castable = magics.get(&row.0).is_some_and(|d| knowledge >= d.difficulty);
        bg.0 = if selected_magic.id.as_deref() == Some(row.0.as_str()) {
            SLOT_SEL
        } else if castable {
            SLOT_BG
        } else {
            SLOT_DISABLED
        };
    }
    // Row text.
    for (rowtext, mut t, mut color) in &mut sets.p3() {
        if let Some(d) = magics.get(&rowtext.0) {
            let sym = if d.symbol.is_empty() { "◇" } else { d.symbol.as_str() };
            **t = format!("{sym} {}   MP{}  必要{}", d.name, d.mp_cost, d.difficulty);
            color.0 = if knowledge >= d.difficulty {
                Color::srgb(0.9, 0.9, 0.85)
            } else {
                Color::srgb(0.5, 0.5, 0.5)
            };
        }
    }
    // Detail.
    {
        let mut p = sets.p4();
        if let Ok(mut t) = p.get_single_mut() {
            **t = selected_magic
                .id
                .as_ref()
                .and_then(|id| magics.get(id))
                .map(|d| {
                    let dur = if d.duration_cycles == 0 {
                        "永続/即時".to_string()
                    } else {
                        format!("{}サイクル", d.duration_cycles)
                    };
                    let extra = if d.is_attack() {
                        format!("\n攻撃魔法(光弾{})", d.projectiles)
                    } else if d.liquefiable {
                        "\n液体化できる".to_string()
                    } else {
                        String::new()
                    };
                    format!(
                        "『{}』\n{}\nMP {} / 難易度 {} / 持続 {}{}",
                        d.name,
                        if d.description.is_empty() { "(説明なし)" } else { &d.description },
                        d.mp_cost,
                        d.difficulty,
                        dur,
                        extra,
                    )
                })
                .unwrap_or_else(|| "魔法を選択".to_string());
        }
    }
    // Target-tab highlight.
    for (btn, mut bg) in &mut sets.p5() {
        bg.0 = if btn.0 == selected_magic.ally_target { TAB_SEL } else { BTN_BG };
    }
}

/// Handle clicks on item-view widgets: tabs, slots, and the item action buttons
/// (equip / unequip / eat / read / drink / place / ZZZ).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn data_screen_interactions(
    screen: Res<DataScreen>,
    mut party: ResMut<Party>,
    catalog: Res<ItemCatalog>,
    magics: Res<MagicCatalog>,
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
            ActionKind::Read => {
                let idef = member
                    .inventory
                    .get(slot)
                    .and_then(|it| catalog.get(&it.def_id))
                    .cloned();
                let Some(idef) = idef else {
                    log.push("スロットが空だ");
                    continue;
                };
                if idef.kind != crate::item::ItemKind::Scroll {
                    log.push("巻物ではない");
                    continue;
                }
                if let Some(mid) = &idef.teaches
                    && let Some(mdef) = magics.get(mid)
                    && !mdef.description.is_empty()
                {
                    log.push(mdef.description.clone());
                }
                match crate::magic::learn_scroll(member, &idef, &magics, &catalog) {
                    Ok(msg) => {
                        member.inventory.take(slot);
                        selected_slot.slot = None;
                        log.push(msg);
                    }
                    Err(e) => log.push(e),
                }
            }
            ActionKind::Drink => match crate::magic::drink_potion(member, slot, &magics, &catalog, &rules.hunger) {
                Ok(msg) => log.push(msg),
                Err(e) => log.push(e),
            },
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

/// Handle clicks on magic-view widgets: view tabs, spell rows, target tabs, and
/// the cast / liquefy buttons.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn data_magic_interactions(
    screen: Res<DataScreen>,
    mut view: ResMut<DataView>,
    mut party: ResMut<Party>,
    catalog: Res<ItemCatalog>,
    magics: Res<MagicCatalog>,
    selected: Res<SelectedMember>,
    mut selected_magic: ResMut<SelectedMagic>,
    mut log: ResMut<MessageLog>,
    mut cast_ev: EventWriter<CastMagic>,
    view_tabs: Query<(&Interaction, &ViewTab), Changed<Interaction>>,
    rows: Query<(&Interaction, &MagicRow), Changed<Interaction>>,
    targets: Query<(&Interaction, &MagicTargetBtn), Changed<Interaction>>,
    acts: Query<(&Interaction, &MagicActBtn), Changed<Interaction>>,
) {
    if !screen.open {
        return;
    }
    for (interaction, tab) in &view_tabs {
        if *interaction == Interaction::Pressed {
            view.magic = tab.0;
        }
    }
    for (interaction, row) in &rows {
        if *interaction == Interaction::Pressed {
            selected_magic.id = Some(row.0.clone());
            selected_magic.ally_target = selected.index;
        }
    }
    for (interaction, tb) in &targets {
        if *interaction == Interaction::Pressed {
            selected_magic.ally_target = tb.0;
        }
    }
    for (interaction, act) in &acts {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(id) = selected_magic.id.clone() else {
            log.push("魔法を選択してください");
            continue;
        };
        let Some(def) = magics.get(&id).cloned() else {
            continue;
        };
        match act.0 {
            MagicAct::Cast => {
                let target = if def.is_attack() {
                    CastTarget::FrontEnemy
                } else if matches!(def.kind, crate::magic::MagicKind::Revive { .. }) {
                    CastTarget::DownedAuto
                } else {
                    CastTarget::Member(selected_magic.ally_target)
                };
                cast_ev.send(CastMagic { caster: selected.index, magic_id: id, target });
            }
            MagicAct::Liquefy => {
                let Some(member) = party.members.get_mut(selected.index) else {
                    continue;
                };
                match crate::magic::liquefy(member, &def, &catalog) {
                    Ok(msg) => log.push(msg),
                    Err(e) => log.push(e),
                }
            }
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
