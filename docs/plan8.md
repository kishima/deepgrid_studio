# plan8: イベントとギミック

> **先行詳細化に関する注意**: plan5〜7 完了前に書かれた計画。着手時に
> 前提を実際のリポジトリ状態と照合し、この文書を更新してから始めること。

## この文書について

第8実装計画書。仕様源は [dandan_spec_event.md](dandan_spec_event.md)(全面的に)
と [dandan_spec_mapeditor.md](dandan_spec_mapeditor.md)(ギミック・連絡通路)、
[plan2.md](plan2.md)「追補」(穴・ドア初期状態・ホロスコープ12方向)。

## ゴール

ダンジョンを「仕掛けのある迷宮」にする。イベントシステム(フラグ・遅延・
14種のアクション)とトリガー(鍵穴・スイッチ・しかけ床・ワープポイント)、
plan2 追補の地形拡張、レベル間移動(連絡通路)を実装する。

## 現状(想定: plan7 完了時点)

- アイテム・モンスター・魔法が動作。プロジェクト形式 v5。
- ドアは kind 単位の開閉のみ(Space で正面をトグル)。
- レベルは常に levels[0] だけがプレイされる。

## スコープ

### やること

1. イベントデータモデル(トリガー、フラグ条件、遅延、アクション14種)
2. イベントフラグ(既定64個、LimitsConfig で拡張可)と実行キュー
3. トリガーブロック: 鍵穴・スイッチ(4形態)、しかけ床(3条件)、
   ワープポイント(隠し属性)
4. 地形拡張: 穴ブロック、ドアの初期開閉状態、書ける壁、
   ホロスコープの上下方向(12方向の一部)
5. アクション実装: ワープ/水位変更/パーティ復活/モンスター発生/
   アイテム発生/ブロック発生/ドア開閉/移動モード設定/スイッチ操作/
   フラグ変更/連結終了・ループ(+BGM変更とデモ起動は**スタブ**、plan10で実体化)
6. 連絡通路(レベル間移動)とマルチレベルプレイ
7. マップエディターへの最小限の配置対応(トリガー+地形拡張ブロック)
8. 検証シーンとユニットテスト

### やらないこと(後続planへ)

- イベントエディターUI(パラメーター編集はRON手書き) → plan9
- BGM変更・デモ起動の実体 → plan10(アクションの枠と発火ログだけ作る)
- 鉛筆で壁に書く操作の入力UI(書ける壁の表示のみ実装、書き込みは
  データがあれば表示する形) → plan9 で入力UI

## データモデル

`src/event.rs`(新設):

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TriggerKind {
    Keyhole { key_item: String },       // 鍵穴: 指定アイテム使用で発動
    SwitchOneWay,                       // 戻らない(一度ONになったら固定)
    SwitchToggle,                       // トグル
    SwitchPush,                         // プッシュ(押している間/押した瞬間)
    FloorPlate { condition: PlateCond },// しかけ床
    WarpPoint { hidden: bool },
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum PlateCond { Step, Weight { min_x100g: i32 }, ItemPlaced { item: Option<String> } }

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum FlagJoin { And, Or }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FlagCond { pub flag: usize, pub must_be_on: bool }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum EventAction {
    Warp { level: usize, x: i32, y: i32, floor: usize, facing: Facing },
    SetLiquid { level: usize, floor: usize, x: i32, y: i32, kind: Option<Block> }, // None=抜く
    ChangeBgm { bgm: String },          // plan10 まで発火ログのみ
    ReviveParty,
    SpawnMonster { monster: String, x: i32, y: i32, floor: usize },
    SpawnItem { item: String, x: i32, y: i32, floor: usize },
    SetBlock { x: i32, y: i32, floor: usize, block: Block },
    StartDemo { demo: String },         // plan10 まで発火ログのみ
    SetMoveMode { mode: MoveMode },     // Normal / Free(空中歩行) / Locked
    OperateSwitch { event: String, on: bool },
    SetFlag { flag: usize, on: bool },
    EndChain,                            // 連結終了
    Loop,                                // 先頭アクションへ戻る(遅延を挟んで)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EventDef {
    pub id: String,
    pub trigger: TriggerKind,
    pub at: (i32, i32, usize),          // 設置座標(トリガーブロック位置)
    pub delay_cycles: u64,              // 実行遅延(参考: 0〜63)
    pub flags: Vec<FlagCond>,           // 参照フラグ
    pub join: FlagJoin,                 // AND / OR
    pub actions: Vec<EventAction>,      // 順に実行(連結)
}
```

- レベルファイルに `events: [EventDef]` を追加(プロジェクト形式 v6)。
- 実行時: `EventFlags`(既定64個。`limits.event_flags`)、
  `EventQueue`(発火済みイベントの遅延カウントダウンをサイクル駆動で処理)。
- Loop は「actions 先頭から再実行(delay を再適用)」。無限ループ前提の
  演出用。EndChain はそこで打ち切り(条件分岐的に使う)。

## 地形拡張(plan2 追補の取り込み)

- `Block::Hole`: 床を張らない・進入すると支持を無視して落下。
  マップ文字 `o`。
- **ドア初期状態**: マップ文字 `1`/`2` は「閉」、`!`/`@` を「開」として追加
  (kind0開/kind1開)。DoorStates の初期値をマップから構成する。
- `Block::WritableWall { text: String }`: 書ける壁。正面で「見る」と本文を
  メッセージ表示。編集入力UIは plan9(データにあれば表示)。
- ホロスコープ上下: `pass_from` に Up/Down を追加(はしご以外の垂直移動
  制御。落下・はしごの通過判定に組み込む)。12方向の斜め等は対象外のまま。

## 連絡通路(レベル間移動)

- project.ron の levels 配列が複数レベルを持てるのは既存。
  `EventAction::Warp` の level 指定でレベルを跨ぐ。
- 「連絡通路」は階段の見た目を持つ専用ブロック `Block::Stairs { up: bool }`
  として実装し、進入すると対応する接続先(レベルファイルの
  `stairs_links: [(from(x,y,floor), to_level, to(x,y,floor,facing))]`)へ遷移。
- レベル遷移時: 現レベルのモンスター状態・ドア状態・落ちたアイテムを
  メモリ上に保持(レベルごとの RuntimeState マップ)。戻ったとき復元される。
  フラグはゲーム全体で共有。
- sample プロジェクトに level01 を追加し、階段で行き来できるようにする。

## エディター対応(最小限)

plan3 の2Dマップエディターに以下を追加(フルのイベント編集は plan9):

- パレット追加: 穴、開状態ドア、階段(上下)、書ける壁、
  鍵穴/スイッチ/しかけ床/ワープポイント(トリガー種別はプレースホルダで
  設置のみ。パラメーターは RON 手書き)
- events の座標とパレット設置座標の整合チェック(保存時に警告)

## 実装ステップ

1. データモデル+v6ローダー+ラウンドトリップテスト
2. フラグ・キュー・遅延・AND/OR(ユニットテスト: 条件評価と遅延)
3. トリガーブロックの設置・表示・作動(鍵穴はアイテム使用、
   しかけ床は重量計算に plan5 の総重量を利用)
4. アクション実装(スタブ2種含む)。SetBlock/SetLiquid は描画の
   再構築が必要 → dungeon_mesh をセル単位で更新できるようにする
5. 地形拡張(穴・ドア初期状態・書ける壁・ホロスコープ上下)
6. 連絡通路とレベル間 RuntimeState
7. エディターのパレット追加
8. 検証シーン: `plate`(しかけ床でモンスター出現)、`warp`(ワープ後の
   位置)、`stairs`(level01 側で撮影)、`hole`(穴から落下)

## 受け入れ基準

1. ビルド完走、clippy/test 通過。
2. `DEEPGRID_DEBUG_SHOT=plate|warp|stairs|hole` 撮影+既存シーン全通過。
3. 手動確認: スイッチ4形態の挙動差、フラグAND/OR、遅延、ループイベント、
   鍵穴に鍵を使う、しかけ床の重量条件、隠しワープ、書ける壁の閲覧、
   階段でレベルを往復して状態が保持されること。
4. sample の RON だけで全ギミックのデモが再現できる。

## 実装上の注意

- イベント処理は必ずサイクル駆動(clock.rs)。フレーム依存禁止。
- SetBlock で通行不能化した場所にプレイヤー/モンスターがいる場合は
  押し出さず「埋まったら1フロア上へ落下扱い」等の単純ルールを決めて
  文書に追記する。
- フラグ番号は 0-based。エディター表示は 0〜63(既定)。
- 発火ログ(どのイベントがいつ発火したか)を debug ログに出す
  (デバッグの主要手段になる)。
- スタブ(BGM/デモ)はメッセージウインドーに「♪BGMが変わった(未実装)」
  等を出して発火が分かるようにする。
