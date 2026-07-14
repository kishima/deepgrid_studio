# plan8: イベントとギミック

## この文書について

第8実装計画書(2026-07-14、plan7 完了後の実状に合わせて詳細化済み)。
仕様源は [dandan_spec_event.md](dandan_spec_event.md)(全面的に)、
[dandan_spec_mapeditor.md](dandan_spec_mapeditor.md)(ギミック・連絡通路)、
[plan2.md](plan2.md)「追補」(穴・ドア初期状態・ホロスコープ上下)。
開発環境の制約は [plan1.md](plan1.md)。数値レンジは参考値。

## ゴール

ダンジョンを「仕掛けのある迷宮」にする: イベント(フラグ・遅延・アクション)、
トリガー(鍵穴・スイッチ・しかけ床・ワープ)、地形拡張(穴・開始開状態ドア・
書ける壁・上下ホロスコープ)、そして連絡通路による**レベル間移動**。

## 現状(plan7 完了時点の実状)

- プロジェクト形式 **v5**(`PROJECT_VERSION = 5`)。magics.ron まで込み。
- `Block` は **Copy な enum**(Wall/Empty/Water/Fire/Poison/Ladder/
  Door{kind}/Horoscope{pass_from: Facing})。マップ文字変換は
  **src/project.rs の `char_to_block` / 逆変換**(loader.rs は存在しない)。
  現行文字: `# . ~ ^ % H 1 2 < > n v`。
- ダンジョンメッシュは `render/dungeon_mesh.rs` の `setup_dungeon` が
  **起動時に一括生成**(セル単位エンティティだが再構築の仕組みなし)。
  ドアのみ `DoorTile`+Visibility で開閉に追従。
- **levels[0] 固定ロード**(main.rs)。レベル間移動の仕組みなし。
- サイクル駆動基盤(clock.rs `CycleTick`)、`GameRng`(固定シード対応)、
  `RulesConfig`(rules.rs)、メッセージ/autotest(**33ステップ**)確立済み。
- しかけ床の重量条件に使える総重量計算は plan5 の
  `Inventory::total_weight`(パーティ合計は足せばよい)。
- エディター(egui)は plan3 の2D編集+基本パレットのみ。

## スコープ

### やること

1. イベントデータモデル(トリガー/フラグ条件 AND・OR/遅延/アクション列)
   とプロジェクト形式 **v6**
2. イベントフラグ(既定64、`limits.event_flags`)と実行キュー(サイクル駆動)
3. トリガーブロック: 鍵穴、スイッチ(戻らない/トグル/プッシュ)、
   しかけ床(踏む/重量/アイテム設置)、ワープポイント(隠し属性)
4. 地形拡張: 穴、ドアの初期開状態、書ける壁(閲覧のみ)、上下ホロスコープ
5. アクション実装: ワープ/水位変更/パーティ復活/モンスター発生/アイテム発生/
   ブロック発生/ドア開閉/移動モード設定/スイッチ操作/フラグ変更/連結終了/
   ループ(**BGM変更とデモ起動はスタブ**: メッセージ+発火ログのみ、plan10 で実体化)
6. **ダンジョンメッシュの部分再構築**(SetBlock/SetLiquid のための基盤)
7. 連絡通路(階段ブロック)とマルチレベル+レベルごとの状態保持
8. エディターへの最小対応(新ブロックのパレット追加。イベント編集は plan9)
9. autotest 9ステップ追加+ユニットテスト+検証シーン

### やらないこと(後続planへ)

- イベントエディターUI(パラメーターはRON手書き) → plan9
- BGM・デモの実体 → plan10
- 書ける壁への**書き込み**UI(鉛筆アイテム連動) → plan9
  (plan8 はデータにある文章の閲覧のみ)
- ホロスコープの斜め方向(12方向の残り) → 必要になったとき
- 秘薬・モンスター発生イベントのエディター配置 → plan9

## データモデル

### 地形(Block は Copy を維持する)

```rust
pub enum Block {
    // ...既存...
    /// 穴: 支持があっても床を張らず、進入すると落下(plan2追補)。
    Hole,
    /// 連絡通路(階段)。進入すると stairs_links の接続先レベルへ遷移。
    Stairs { up: bool },
    /// 書ける壁。本文は LevelData.wall_texts に持つ(Block の Copy 維持のため)。
    WritableWall,
    /// 上下ホロスコープ: 垂直方向の一方通行。from_below=true なら
    /// 下からの通過(はしご上り・落下抜けの禁止側に注意)のみ許す。
    HoroscopeVert { from_below: bool },
    /// トリガーブロック。パラメーターは events 側に座標で紐づく。
    Keyhole,
    Switch,
    FloorPlate,
    WarpPoint,
}
```

- マップ文字の追加: `o`=Hole `u`/`d`=Stairs(上/下) `W`=WritableWall
  `^v` は使用済みのため上下ホロスコープは `A`(下から上OK)/`V`(上から下OK)、
  `K`=Keyhole `S`=Switch `P`=FloorPlate `T`=WarpPoint、
  `!`=Door kind0 の初期開 `@`=Door kind1 の初期開。
  `char_to_block`/逆変換(project.rs)を対で更新し、ラウンドトリップ
  テストの全文字リストにも追加する。
- ドア初期開状態: ロード時に `!`/`@` を見つけたら DoorStates の初期値を
  開にする(ブロックとしては既存 Door{kind} と同一。glyph は保存時に
  DoorStates の**初期値**を見て書き分ける)。
- LevelData 追加(すべて serde default):
  - `wall_texts: Vec<(i32, i32, usize, String)>`(x,y,floor,本文)
  - `stairs_links: Vec<StairsLink>`
    `StairsLink { from: (i32,i32,usize), to_level: usize, to: (i32,i32,usize), to_facing: Facing }`
  - `events: Vec<EventDef>`

### イベント

```rust
pub enum TriggerKind {
    Keyhole { key_item: String },        // 鍵は消費しない(暫定)
    SwitchOneWay,                        // 一度ONで固定
    SwitchToggle,
    SwitchPush,                          // 押した瞬間のみ発火(状態を持たない)
    FloorPlate { cond: PlateCond },
    WarpPoint { hidden: bool },          // hidden はマーカー非表示
    None,                                // トリガー無し(OperateSwitch 専用の的)
}
pub enum PlateCond { Step, Weight { min_x100g: i32 }, ItemPlaced { item: Option<String> } }

pub struct FlagCond { pub flag: usize, pub must_be_on: bool }
pub enum FlagJoin { And, Or }

pub enum EventAction {
    Warp { level: usize, x: i32, y: i32, floor: usize, facing: Facing },
    SetLiquid { x: i32, y: i32, floor: usize, kind: Option<LiquidKind> }, // None=抜く
    ChangeBgm { bgm: String },           // スタブ(plan10)
    ReviveParty,                         // 全員 down解除+HP全快(オリジナル準拠)
    SpawnMonster { monster: String, x: i32, y: i32, floor: usize },
    SpawnItem { item: String, x: i32, y: i32, floor: usize },
    SetBlock { x: i32, y: i32, floor: usize, block: Block },
    StartDemo { demo: String },          // スタブ(plan10)
    SetMoveMode { mode: MoveMode },      // Normal / Free(空中歩行) / Locked
    OperateSwitch { event: String, on: bool },
    SetFlag { flag: usize, on: bool },
    EndChain,
    Loop,                                // 先頭へ(delay を再適用)
}

pub struct EventDef {
    pub id: String,
    pub trigger: TriggerKind,
    pub at: (i32, i32, usize),           // トリガーブロックの座標
    #[serde(default)] pub delay_cycles: u64,      // 参考: 0〜63
    #[serde(default)] pub flags: Vec<FlagCond>,
    #[serde(default)] pub join: FlagJoin,          // 既定 And
    pub actions: Vec<EventAction>,
}
```

- 実行時リソース: `EventFlags`(Vec<bool>、`limits.event_flags`)、
  `EventQueue`(発火済み: (level, event id, 発火サイクル+delay, 次action index))。
  すべて `CycleTick` 駆動。1サイクルに複数アクションを進めてよいが、
  Loop は必ず delay を挟む(0 でも1サイクル待つ — 無限ループでフリーズ
  しないための規約)。
- 条件評価: flags が空なら常に成立。And/Or は must_be_on との一致で判定。
- 発火ログ: `info!` で「event fired: id (trigger)」を必ず出す(デバッグの
  主要手段)。スタブは加えてメッセージ「♪BGMが〜(未実装)」等。

### 挙動の要点

- **鍵穴**: 正面の Keyhole に Space → パーティの誰かが key_item を所持で
  発火(SwitchOneWay 同様、一度使うと再発火しない)。鍵は消費しない(暫定。
  変える場合は本文書を更新)。所持なし→「かぎあなが ある。あう鍵がない」。
- **スイッチ**: 正面に Space。OneWay=初回のみ/Toggle=ON・OFF交互
  (OFF時はアクション列を**再実行しない**、フラグ SetFlag だけ反転…は
  複雑なので、**Toggle は ON/OFF どちらへの切替でもアクション列を実行**し、
  作り手がフラグ条件で分岐する、を規約とする)/Push=押すたび発火。
- **しかけ床**: プレイヤーがそのタイルに進入したサイクルに評価。
  Weight はパーティ全員の総重量(×100g)合計 ≥ min。ItemPlaced は
  そのタイルに床アイテム(指定 id、None=何でも)が置かれた時
  (handle_place 後にチェック。置いた瞬間に発火)。
- **ワープポイント**: 進入で即 Warp アクション(EventDef の actions を実行)。
  hidden: true は3D表示なし(見えない罠)。false は薄く光るマーカー。
- **穴**: 進入→支持無視で落下(既存の landing_floor 探索から開始フロアを
  変えるだけ)。Hole は「その場で下が抜けている」ので Hole セル自体に
  立つことはない。
- **SetBlock/SetLiquid**: ダンジョンデータ(Dungeon リソース)を書き換え、
  `TileDirty { x, y, floor }` イベントで**そのセル+隣接6方向**のメッシュを
  再構築する(setup_dungeon をセル単位関数に分解し、`TilePos` マーカー付き
  エンティティを despawn→respawn)。プレイヤー/モンスターのいるセルを
  Wall にされた場合: 1フロア上へ「押し出し落下」(埋まり防止の単純規約)。
- **移動モード**: `MoveMode` リソース。Free は支持判定を無視(落下しない。
  はしご不要で昇降可)、Locked は移動系 Command を全拒否(イベント演出用)。
  エディターの自由移動(plan9)もこれを使う。

## 連絡通路とマルチレベル

- `CurrentLevel(usize)` リソース新設。レベルに属すエンティティ
  (メッシュ、床アイテム、モンスター、トリガーマーカー)へ `LevelScoped`
  マーカーを付け、遷移時に一括 despawn → 遷移先を構築。
- **RuntimeState を Level ごとに保持**: `LevelStates: HashMap<usize, LevelState>`
  - LevelState: モンスター状態(hp/pos/死亡・再生タイマー)、床アイテムの
    現状(初期配置との差分ではなく**全量スナップショット**でよい)、
    DoorStates、発火済みトリガー(OneWay/鍵穴の消費状態)、水位変更などの
    Block 差分(`Vec<((x,y,floor), Block)>`)
  - 遷移時: 現レベルを保存 → 遷移先があれば復元、なければ初期構築。
  - フラグ・MoveMode・パーティはグローバル(保持しない)。
- 階段: Stairs へ進入 → stairs_links から接続を引く(無ければ
  「くずれていて 通れない」)→ フェード等はなしで即遷移+
  メッセージ「階段を のぼった/おりた」。
- sample に **level01.ron** を追加(小さめ。階段で往復、ワープの的、
  SetBlock で開く隠し部屋、といった plan8 機能のショーケース)。
  project.ron の levels を2件に。

## エディター対応(最小限)

- パレットに新ブロック(o u d W A V K S P T ! @)を追加(色+記号表示は
  plan3 の仕組みに追随)。
- events / wall_texts / stairs_links は RON 手書き(plan9 でUI化)。
  保存時、トリガーブロックの座標に対応する EventDef が無い場合は
  ステータスバーに警告(逆も同様)。

## autotest 追加ステップ(34〜42)

- 34 `plate-step`: しかけ床(Step)を踏む → SpawnMonster が発火し
  指定座標にモンスターが現れる
- 35 `flags-andor`: フラグ条件 And(不成立→発火しない)→ SetFlag 後に
  成立して発火。Or も1ケース
- 36 `delay-loop`: delay 付きイベントが指定サイクル後に実行される。
  Loop イベントが2周以上実行され、EndChain で止まるイベントは1周で止まる
- 37 `keyhole`: 鍵なし→不発+メッセージ、鍵所持→ドア開(DoorStates)+
  再使用しても再発火しない
- 38 `switch-forms`: OneWay/Toggle/Push の発火回数の差をアサート
- 39 `warp-hidden`: 隠しワープ進入 → 座標・向きが跳んでいる
- 40 `setblock`: SetBlock で通路が壁になり移動が拒否される → SetLiquid で
  水になり hazard が作動する(既存の水ステップ流用)
- 41 `stairs-state`: level01 へ移動 → level00 に戻る → 置いたアイテムと
  倒したモンスターの状態が保持されている
- 42 `hole-and-vert`: 穴で落下する。上下ホロスコープの禁止方向で
  はしご移動が拒否される

(トリガーやフラグの直接操作は不可。すべて実プレイ操作
(ScriptedInput/イベント)経由。座標は autotest 用に level00/01 に
専用の小部屋を用意してよい — 既存33ステップのエリアと干渉しないこと)

## 実装ステップ

1. Block 拡張+文字変換+ラウンドトリップテスト(v6)
2. メッシュの部分再構築(TileDirty。SetBlock より先に単体で作る —
   ドア初期開・水位変更の下地)
3. イベント基盤(フラグ/キュー/遅延/AND-OR/Loop/EndChain。
   条件評価と遅延はユニットテスト)
4. トリガー(鍵穴/スイッチ3形態/しかけ床3条件/ワープ)
5. アクション残り(SetBlock/SetLiquid/Spawn系/ReviveParty/MoveMode/
   OperateSwitch/スタブ2種)
6. 地形拡張(穴・初期開ドア・書ける壁閲覧・上下ホロスコープ)
7. マルチレベル(CurrentLevel/LevelScoped/LevelStates)+level01
8. エディターパレット+保存時警告
9. autotest 34〜42+検証シーン(`plate|warp|stairs|hole`)

## 受け入れ基準

1. ビルド完走、clippy 警告なし、`cargo test` 全通過。
2. `DEEPGRID_AUTOTEST=1` が **42ステップ全PASS**・終了コード0。
3. `DEEPGRID_DEBUG_SHOT=plate|warp|stairs|hole` が撮影でき(mtime確認)、
   既存13シーンも全部通る。
4. 手動確認(操作感のみ): 仕掛けの発見と作動の手触り、レベル遷移の
   違和感、隠しワープの「見えなさ」。
5. sample(level00+level01)のデータだけで全ギミックを体験できる。

## 実装上の注意

- Block の **Copy を壊さない**(文字列を持つ変種を作らない。本文・
  パラメーターは LevelData/EventDef 側に座標で紐づける)。
- イベント処理は CycleTick 駆動。Loop の最低1サイクル規約を守る
  (無限ループ防止)。
- 発火済み状態(OneWay/鍵穴)は LevelState に含めてレベル往復で保持。
- SetBlock 系のメッシュ再構築は「セル+隣接」だけにとどめ、全再構築を
  しない(40×40×5 で目に見えて止まるため)。
- 既存 autotest(33ステップ)のエリア・前提を壊さない(新ギミックの
  サンプル配置は既存の動線の外に)。
- `char_to_block` と逆変換・エディターパレット・ラウンドトリップテストの
  文字リストは**必ず同時に**更新する(1箇所でも漏れると保存で壊れる)。
- rand/GameRng・`ClusterConfig::Single`・UI棲み分けの既存規約は維持。
