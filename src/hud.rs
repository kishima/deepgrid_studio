//! Main-screen HUD (plan4): the status window (party cards: portrait + name +
//! HP/MP/concentration bars) and the message window (latest lines of an event
//! log). Built with **bevy_ui**, not egui — bevy_ui draws through the Bevy
//! render graph so it appears in `Screenshot` captures (plan3 Step 6), which the
//! play-screen verification depends on.
//!
//! Japanese text uses PixelMplus (bevy's `default_font` has no Japanese glyphs).
//! Bar colours follow the original: HP blue, MP red, concentration green
//! (project.md「メイン画面」). The layout is anchored with absolute positioning so
//! it follows window resizes.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::character::Party;
use crate::portrait::Portraits;

/// Regular / bold Japanese pixel font (M+; see CREDITS.md).
const FONT_REGULAR: &str = "fonts/PixelMplus12-Regular.ttf";
const FONT_BOLD: &str = "fonts/PixelMplus12-Bold.ttf";

/// Ring-buffer capacity for the event log.
const LOG_CAPACITY: usize = 256;
/// How many of the most recent lines the message window shows.
const VISIBLE_LINES: usize = 4;

/// Sidebar width in px.
const SIDEBAR_W: f32 = 220.0;
/// Message window height in px.
const MESSAGE_H: f32 = 96.0;
/// Bar track width in px.
const BAR_W: f32 = 120.0;

const HP_COLOR: Color = Color::srgb(0.25, 0.55, 1.0);
const MP_COLOR: Color = Color::srgb(0.95, 0.30, 0.30);
const CONC_COLOR: Color = Color::srgb(0.35, 0.85, 0.35);
const BAR_TRACK: Color = Color::srgb(0.12, 0.12, 0.15);
const CARD_BG: Color = Color::srgba(0.08, 0.08, 0.11, 0.92);
const CARD_BG_DOWN: Color = Color::srgba(0.14, 0.14, 0.14, 0.92);
const PANEL_BG: Color = Color::srgba(0.05, 0.05, 0.07, 0.90);

/// Event log (project.md「メッセージウインドー」). All gameplay text routes through
/// `push`, which appends to a bounded ring buffer; the window shows the last
/// [`VISIBLE_LINES`].
#[derive(Resource, Default)]
pub struct MessageLog {
    lines: VecDeque<String>,
}

impl MessageLog {
    /// Append one line, dropping the oldest past the capacity.
    pub fn push(&mut self, text: impl Into<String>) {
        self.lines.push_back(text.into());
        while self.lines.len() > LOG_CAPACITY {
            self.lines.pop_front();
        }
    }

    /// Whether any retained line contains `needle` (used by the autotest driver).
    pub fn contains(&self, needle: &str) -> bool {
        self.lines.iter().any(|l| l.contains(needle))
    }

    /// The visible lines, oldest-first, padded to [`VISIBLE_LINES`] with blanks
    /// so the window's line rows map 1:1 top→bottom.
    fn visible(&self) -> Vec<&str> {
        let n = self.lines.len();
        let start = n.saturating_sub(VISIBLE_LINES);
        let mut out: Vec<&str> = (start..n).map(|i| self.lines[i].as_str()).collect();
        while out.len() < VISIBLE_LINES {
            out.insert(0, "");
        }
        out
    }
}

/// Seed the log with a greeting so the message window isn't blank on entry.
pub fn greet(mut log: ResMut<MessageLog>) {
    log.push("DeepGrid Studioへようこそ。");
}

/// Which stat a bar's fill node tracks.
#[derive(Clone, Copy)]
enum BarKind {
    Hp,
    Mp,
    Concentration,
}

/// Fill node of a bar; `update_status` sets its width from the party state.
#[derive(Component)]
pub struct StatBar {
    slot: usize,
    kind: BarKind,
}

/// A party card's root node (tinted grey when the member is down).
#[derive(Component)]
pub struct CardRoot {
    slot: usize,
}

/// A card's name text.
#[derive(Component)]
pub struct CardName {
    slot: usize,
}

/// A message-window line, `row` 0 = top (oldest visible).
#[derive(Component)]
pub struct MessageLine {
    row: usize,
}

/// An action-icon button; clicking it fires the same `PlayerAction` as its key.
#[derive(Component)]
pub struct ActionIcon(crate::monster::PlayerAction);

/// A move-icon button; clicking it queues the same movement `Command` as its key.
#[derive(Component)]
pub struct MoveIcon(crate::player::Command);

/// One-slot buffer for a movement command issued by a move-icon click. Consumed
/// by `player_movement` (folded into its `ActionEvents` param).
#[derive(Resource, Default)]
pub struct IconMove(pub Option<crate::player::Command>);

/// Fire `PlayerAction`s from action-icon clicks.
pub fn action_icon_clicks(
    icons: Query<(&Interaction, &ActionIcon), Changed<Interaction>>,
    mut actions: EventWriter<crate::monster::PlayerAction>,
) {
    for (interaction, icon) in &icons {
        if *interaction == Interaction::Pressed {
            actions.send(icon.0);
        }
    }
}

/// Buffer a movement command from move-icon clicks.
pub fn move_icon_clicks(
    icons: Query<(&Interaction, &MoveIcon), Changed<Interaction>>,
    mut icon_move: ResMut<IconMove>,
) {
    for (interaction, icon) in &icons {
        if *interaction == Interaction::Pressed {
            icon_move.0 = Some(icon.0);
        }
    }
}

/// Build the HUD once at startup. No-op for an empty party (v1 projects) — the
/// status window is simply absent, per plan4.
pub fn setup_hud(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    party: Res<Party>,
    portraits: Res<Portraits>,
) {
    if party.is_empty() {
        return;
    }
    let regular: Handle<Font> = asset_server.load(FONT_REGULAR);
    let bold: Handle<Font> = asset_server.load(FONT_BOLD);

    // Status window: right sidebar, one card per member.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                right: Val::Px(8.0),
                width: Val::Px(SIDEBAR_W),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|panel| {
            for (slot, member) in party.members.iter().enumerate() {
                let portrait = portraits.images.get(slot).cloned().unwrap_or_default();
                spawn_card(panel, slot, &member.character.first_name, portrait, &regular, &bold);
            }
        });

    // Message window: bottom strip.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(8.0),
                right: Val::Px(8.0),
                bottom: Val::Px(8.0),
                height: Val::Px(MESSAGE_H),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                padding: UiRect::all(Val::Px(8.0)),
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|panel| {
            for row in 0..VISIBLE_LINES {
                panel.spawn((
                    Text::new(""),
                    TextFont {
                        font: regular.clone(),
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.85, 0.85, 0.80)),
                    MessageLine { row },
                ));
            }
        });

    // Action-icon window: bottom-right, above the message window (plan6). Same
    // functions as the Space/B/C/T/V keys.
    use crate::monster::PlayerAction;
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(8.0),
                bottom: Val::Px(MESSAGE_H + 14.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|row| {
            for (act, label) in [
                (PlayerAction::Attack, "攻撃"),
                (PlayerAction::Guard, "防ぐ"),
                (PlayerAction::Concentrate, "精神"),
                (PlayerAction::Throw, "投げる"),
                (PlayerAction::Steal, "盗む"),
            ] {
                row.spawn((
                    Button,
                    Node { padding: UiRect::axes(Val::Px(7.0), Val::Px(4.0)), ..default() },
                    BackgroundColor(Color::srgb(0.22, 0.24, 0.30)),
                    ActionIcon(act),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new(label),
                        TextFont { font: bold.clone(), font_size: 13.0, ..default() },
                        TextColor(Color::WHITE),
                    ));
                });
            }
        });

    // Move-icon window: bottom-left, above the message window (auxiliary input).
    use crate::player::{Action, Command};
    let grid: [[(Command, &str); 3]; 3] = [
        [(Command::Move(Action::TurnLeft), "左"), (Command::Move(Action::Forward), "↑"), (Command::Move(Action::TurnRight), "右")],
        [(Command::Move(Action::StrafeLeft), "←"), (Command::Move(Action::Backward), "↓"), (Command::Move(Action::StrafeRight), "→")],
        [(Command::ClimbUp, "上"), (Command::ClimbDown, "下"), (Command::Get, "取")],
    ];
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(8.0),
                bottom: Val::Px(MESSAGE_H + 14.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                padding: UiRect::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
        ))
        .with_children(|col| {
            for r in grid {
                col.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(3.0), ..default() })
                    .with_children(|row| {
                        for (cmd, label) in r {
                            row.spawn((
                                Button,
                                Node {
                                    width: Val::Px(30.0),
                                    height: Val::Px(26.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.20, 0.22, 0.28)),
                                MoveIcon(cmd),
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Text::new(label),
                                    TextFont { font: bold.clone(), font_size: 14.0, ..default() },
                                    TextColor(Color::WHITE),
                                ));
                            });
                        }
                    });
            }
        });
}

/// Spawn one party card: portrait, name, and the three bars.
fn spawn_card(
    panel: &mut ChildBuilder,
    slot: usize,
    name: &str,
    portrait: Handle<Image>,
    regular: &Handle<Font>,
    bold: &Handle<Font>,
) {
    panel
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(CARD_BG),
            CardRoot { slot },
        ))
        .with_children(|card| {
            // Portrait (3D render target).
            card.spawn((
                Node {
                    width: Val::Px(56.0),
                    height: Val::Px(56.0),
                    ..default()
                },
                ImageNode::new(portrait),
            ));
            // Name + bars column.
            card.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                flex_grow: 1.0,
                ..default()
            })
            .with_children(|col| {
                col.spawn((
                    Text::new(name.to_string()),
                    TextFont {
                        font: bold.clone(),
                        font_size: 15.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.95, 0.90)),
                    CardName { slot },
                ));
                spawn_bar(col, slot, BarKind::Hp, "HP", HP_COLOR, regular);
                spawn_bar(col, slot, BarKind::Mp, "MP", MP_COLOR, regular);
                spawn_bar(col, slot, BarKind::Concentration, "集", CONC_COLOR, regular);
            });
        });
}

/// A labelled bar: a small caption plus a track holding a coloured fill node.
fn spawn_bar(
    col: &mut ChildBuilder,
    slot: usize,
    kind: BarKind,
    label: &str,
    color: Color,
    regular: &Handle<Font>,
) {
    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(4.0),
        ..default()
    })
    .with_children(|row| {
        row.spawn((
            Text::new(label.to_string()),
            TextFont {
                font: regular.clone(),
                font_size: 12.0,
                ..default()
            },
            TextColor(Color::srgb(0.75, 0.75, 0.72)),
        ));
        // Track.
        row.spawn((
            Node {
                width: Val::Px(BAR_W),
                height: Val::Px(9.0),
                ..default()
            },
            BackgroundColor(BAR_TRACK),
        ))
        .with_children(|track| {
            // Fill (width updated each frame).
            track.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(color),
                StatBar { slot, kind },
            ));
        });
    });
}

fn ratio(current: i32, max: i32) -> f32 {
    if max <= 0 {
        return 0.0;
    }
    (current as f32 / max as f32).clamp(0.0, 1.0) * 100.0
}

/// Reflect party state onto the bars each frame.
pub fn update_status_bars(party: Res<Party>, mut bars: Query<(&StatBar, &mut Node)>) {
    for (bar, mut node) in &mut bars {
        let Some(member) = party.members.get(bar.slot) else {
            continue;
        };
        let (cur, max) = match bar.kind {
            BarKind::Hp => (member.state.hp, member.character.stats.max_hp),
            BarKind::Mp => (member.state.mp, member.character.stats.max_mp),
            BarKind::Concentration => {
                (member.state.concentration, member.character.stats.concentration)
            }
        };
        node.width = Val::Percent(ratio(cur, max));
    }
}

/// Grey a card and append 気絶 to the name when the member is down.
pub fn update_cards(
    party: Res<Party>,
    mut cards: Query<(&CardRoot, &mut BackgroundColor)>,
    mut names: Query<(&CardName, &mut Text)>,
) {
    for (card, mut bg) in &mut cards {
        if let Some(member) = party.members.get(card.slot) {
            bg.0 = if member.state.down { CARD_BG_DOWN } else { CARD_BG };
        }
    }
    for (name, mut text) in &mut names {
        if let Some(member) = party.members.get(name.slot) {
            let base = &member.character.first_name;
            **text = if member.state.down {
                format!("{base}  気絶")
            } else {
                base.clone()
            };
        }
    }
}

/// Copy the latest log lines into the message window's rows (oldest at top).
pub fn update_messages(log: Res<MessageLog>, mut lines: Query<(&MessageLine, &mut Text)>) {
    if !log.is_changed() {
        return;
    }
    let visible = log.visible();
    for (line, mut text) in &mut lines {
        if let Some(s) = visible.get(line.row) {
            **text = s.to_string();
        }
    }
}
