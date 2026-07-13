# plan6: モンスターと戦闘

## この文書について

第6実装計画書(2026-07-13、plan5 完了後の実状に合わせて詳細化済み。
先行版の「照合してから着手」注意は解消済み — 本文書が現状ベース)。
仕様源は [dandan_spec_things_editor.md](dandan_spec_things_editor.md)「Enemy」と
[project.md](project.md)(戦闘計算・アクションアイコン)。
開発環境の制約は [plan1.md](plan1.md)「開発環境の前提」。
オリジナルの数値レンジは参考値(project.md「上限値の扱い」)。

## ゴール

モンスターがダンジョンを徘徊し、リアルタイムで戦えるようにする。
アクションアイコンウインドーと移動アイコンウインドーを加えて
project.md「メイン画面」の5ウインドー構成を完成させ、経験値と
レベルアップ(成長タイプ)まで一巡させる。

## 現状(plan5 完了時点の実状)

- プロジェクト形式 **v3**(`PROJECT_VERSION = 3`、src/project.rs)。
  project.ron + levels/(items 配置付き)+ characters.ron + items.ron。
- アイテム: 定義27種(sample)、床表示(floor_items.rs)、拾う(`G`)/置く、
  インベントリ(手2・装備6・ポーチ・リュック、LimitsConfig 由来)、
  装備効果、食べる、重量超過の移動拒否、液体ダメージ(hazard.rs、
  水/火/毒+毒残留 `poison_remaining`)。
- キャラ: character.rs(Stats/StatKind/GrowthType/CharacterState、
  effective_stats、eat)。パーティは `Party` リソース。
- 時間: clock.rs(`GameClock`、`CycleTick` イベント、1サイクル=0.1秒)。
- 入力: movement.rs の `Command` enum
  (Move/ClimbUp/ClimbDown/ToggleDoor/Get/ToggleData)。
  キー使用済み: WASD QE(移動)、R/F(昇降)、Space(ドア)、G(拾う)、
  Tab/I(データ画面)。**B/C/T/V は空いている**。
- UI: hud.rs(ステータス+メッセージ、MessageLog::contains あり)、
  data_screen.rs、portrait.rs。すべて bevy_ui + PixelMplus。
- 検証: `DEEPGRID_DEBUG_SHOT=1|fall|ladder|door|props|items|pickup|data|liquid|editor`。
  **`DEEPGRID_AUTOTEST=1`(autotest.rs、13ステップ)が稼働中** —
  roadmap 横断ルールにより plan6 の新機能もここへステップ追加する。
- src/props.rs: スケルトン2体(minion/warrior)がまだハードコード表示
  (KayKit、アニメはインデックス指定 Idle=40 / Idle_Combat=42)。
  本plan でモンスターシステムに置き換えて props.rs を削除する。
- ユニットテスト19件(ラウンドトリップ、インベントリ等)。

## スコープ

### やること

1. モンスター定義(`monsters.ron`)と配置(プロジェクト形式 **v4**)
2. モンスターの3D表示(KayKitモデル+アニメーション状態機械)と移動AI
3. リアルタイム戦闘(攻撃・防ぐ・精神統一/モンスター攻撃)
4. 命中・ダメージ計算式(すばやさ・持ちやすさ・集中力)
5. アクションアイコンウインドー+移動アイコンウインドー
6. 投擲(プレイヤー、およびモンスターの attack_items 投擲)
7. 盗み(盗みのうで×用心深さ)
8. ドロップ・経験値・レベルアップ(成長タイプ5種)
9. モンスター再生(regen_cycles)、ZZZ休息の「敵接近中は不可」化
10. props.rs の削除(スケルトンをデータ駆動配置へ)
11. **autotest へのステップ追加**(下記)とユニットテスト、検証シーン

### やらないこと(後続planへ)

- 魔法による攻撃・回復、体温・対魔法力の実効果 → plan7
- イベントによるモンスター発生 → plan8
- モンスターエディター → plan9(データは手書きRON)
- 攻撃効果音 → plan10
- **動く大型(2×2)モンスター**(下記「サイズ」の割り切り)

## データモデル

`src/monster.rs`(新設):

```rust
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub enum MoveType { Ground, Air, None }

/// monsters.ron の1エントリ。オリジナルのレンジ(防御0〜32767等)は参考値。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MonsterDef {
    pub id: String,
    pub name: String,
    #[serde(default)] pub profile: String,
    pub max_hp: i32,
    pub attack: i32,
    #[serde(default)] pub defense: i32,
    #[serde(default)] pub agility: i32,       // すばやさ(命中/回避)
    /// 攻撃間隔(サイクル)。小さいほど頻繁。オリジナル準拠で 255 は「攻撃しない」。
    #[serde(default = "default_255")] pub attack_freq: u32,
    #[serde(default)] pub anti_magic: i32,    // 器のみ(plan7)
    #[serde(default)] pub body_temp: i32,     // 器のみ(plan7)
    #[serde(default)] pub resist_air: i32,    // 128以上でその空間を好む
    #[serde(default)] pub resist_water: i32,
    #[serde(default)] pub resist_heat: i32,
    #[serde(default)] pub resist_poison: i32,
    #[serde(default)] pub wariness: i32,      // 用心深さ(盗み判定)
    #[serde(default)] pub regen_cycles: u64,  // 0 = 復活しない
    pub move_type: MoveType,
    #[serde(default)] pub can_use_ladder: bool,
    #[serde(default = "default_true")] pub fits_narrow: bool,
    #[serde(default)] pub sight: i32,         // 0 = 認識しない(〜40目安)
    /// 移動間隔(サイクル)。1が最速、255 は「移動しない」。
    #[serde(default = "default_255")] pub action_unit: u32,
    #[serde(default)] pub flee_hp: i32,
    #[serde(default)] pub large: bool,        // 2×2占有(本planでは不動個体のみ)
    #[serde(default)] pub carry_items: Vec<String>,   // ドロップ/盗み対象
    #[serde(default)] pub attack_items: Vec<String>,  // 投擲攻撃に使う
    /// 撃破時経験値。オリジナルに項目が無いため暫定追加(エディター化は plan9)。
    #[serde(default)] pub exp: i32,
    pub model: String,
    /// アニメーションは glb 内の**名前**で指定(インデックスはパック更新に弱い。
    /// KayKit スケルトンは "Idle" "Walking_A" "1H_Melee_Attack_Slice_Diagonal"
    /// "Hit_A" "Death_A" などを持つ — assets/models/README.md 参照)。
    pub anim: MonsterAnims,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MonsterAnims {
    pub idle: String,
    pub walk: String,
    pub attack: String,
    pub hit: String,
    pub death: String,
}
```

- レベルファイル(v4)に `monsters: [(id, x, y, floor, facing), ...]` を追加。
  バリデーション: 種類数 ≤ `limits.monster_kinds_per_level`、
  配置数 ≤ `limits.monster_placements_per_level`。v3以前は monsters 無しとして
  読める(後方互換は project.rs の既存パターンを踏襲)。
- 実行時: `MonsterState` コンポーネント(hp、GridPos、facing、AIフェーズ、
  次回行動サイクル、死亡サイクル)を持つエンティティ。定義参照は id。

### サイズ(設計判断)

詳細仕様の「大」はスプライト横幅2倍であり2D前提の概念。本作では
`large: true` = **2×2マス占有**と解釈する。ただし plan6 では移動AIの
2×2通行判定が複雑になるため、**大型は不動個体(action_unit=255)のみ
サンプルに配置**し、可動対応は必要になった planで拡張する。占有判定
(進入不可・攻撃対象)は 2×2 の全マスで行う。

## モンスターAI(サイクル駆動)

`CycleTick` を消費し、各個体は「次回行動サイクル ≥ 現在」で1行動:

1. **徘徊**: プレイヤー距離(チェビシェフ) > sight、または視線が壁で
   遮られる(同フロアの単純レイキャスト。斜めは不要、Bresenhamでよい)
   とき: 50% で隣接ランダム移動。
2. **追跡**: 視認したらプレイヤーへ1歩(dx/dy の大きい軸を優先する貪欲法。
   塞がっていればもう一方の軸)。
3. **攻撃**: 隣接時、`attack_freq` サイクル間隔で攻撃。
   attack_items を持つ個体は距離2〜4かつ直線上なら投擲攻撃。
4. **逃走**: hp < flee_hp で遠ざかる1歩(以後視認しても戻らない)。
5. 通行判定: move_type(Ground は支持必須/Air は不要/None は不動)、
   can_use_ladder(はしごマス経由の昇降)、fits_narrow
   (false は左右どちらかが壁のマスに進入しない)、液体
   (該当耐性 < 128 は進入しない。≥128 は好む: 徘徊時の移動先候補で優先)。
   モンスター同士・プレイヤーのマスには進入しない。
6. 見た目: 論理グリッド+補間移動(プレイヤーと同じ思想)。
   歩行中 walk、停止 idle をアニメーション状態機械で切替。

## 戦闘

### 操作(キーとアイコンは同機能)

- **攻撃 `Space`**: 正面1マス。**正面がドアならドア開閉、モンスターなら攻撃**
  に多重化(movement.rs の Space 分岐を拡張)。攻撃者はパーティ内
  ローテーション(生存者が順に振る)。手に武器が無ければ素手。
- **防ぐ `B`**: 次の被攻撃1回のダメージ半減。
- **精神統一 `C`**: 集中力の回復速度を5倍にする(移動・攻撃・被弾で解除)。
- **投げる `T`**: 選択中メンバーの手(左優先)のアイテムを正面へ投擲。
  `important` は不可。射程 = clamp(遠投力/20 + 投げやすさ/50, 1, 6) マス
  (暫定)。直線上最初のモンスター/壁で止まり、モンスターなら
  `するどさ + 投げやすさ/5` 基準のダメージ(命中判定は下式)。
  アイテムは落下地点の床に落ちる。
- **盗む `V`**: 正面のモンスターから
  成功率 = clamp(50 + 盗みのうで − 用心深さ, 5, 95)%(暫定)で
  carry_items から1個取得(モンスターの残所持品から除去)。
  失敗時、対象は即座に反撃してくる(attack_freq を無視して1回攻撃)。
- 新 Command: `Attack, Guard, Concentrate, Throw, Steal`
  (ScriptedInput / autotest / アイコンから共用)。

### 計算式(すべて暫定。変更時はこの文書を更新)

```
命中率(%)   = clamp(50 + (攻すばやさ − 防すばやさ)/10 + 持ちやすさ/10, 10, 95)
基本D        = max(1, するどさ + 攻撃力/10 − 防御力/20)
集中ボーナス  = 基本D × (現在集中力 / max(1, 集中力最大値)) × 0.5
最終D        = round(基本D + 集中ボーナス)   // 攻撃後、攻撃者の集中力は 0
素手: するどさ0・持ちやすさ50。モンスター側も同式(するどさ0)。
被攻撃者はパーティ生存者からランダム。「防ぐ」中は半減(1回で解除)。
```

計算は `src/combat.rs` の純関数に置き、**ユニットテストで境界値**
(clamp両端・素手・防御中)を検証する。

### 死亡・ドロップ・経験値・再生

- モンスター死亡: death 再生 → carry_items の残りを足元へ散らす
  (floor_items::spawn 流用)→ エンティティは残し `dead` 状態に。
  `regen_cycles > 0` なら死亡地点でカウント後、HP全快で復活
  (プレイヤーがそのマスにいる間は延期)。0 なら despawn。
- 経験値 = def.exp を生存メンバーで均等割り(端数切り上げ)。
- **レベルアップ**: 閾値 = 現レベル × 100(暫定)。上昇 = 基礎上昇 ×
  成長係数: 平均型 1.0 / 早期開花 level<20:1.5, 以後0.5 /
  大器晩成 level<20:0.5, 以後1.5 / 天才 1.5 / 才能なし 0.2。
  基礎上昇(暫定): MHP+4、MMP+2、他+1(切り捨て、最低0)。
  実装は `GrowthType::multiplier(level)` として character.rs に置き、
  ユニットテスト対象。
- パーティ全滅: 「全滅した…」+入力ロック(復活は plan7/8)。
  autotest 用に `DEEPGRID_REVIVE=1` で全滅時に即全快復活を許可。

### ZZZ休息の制限

視認距離内(sight 内かつ視線あり)に生存モンスターがいる間は
休息開始を拒否(「モンスターが近くにいる!」)。休息中に接近されたら中断。

## UI

- **アクションアイコンウインドー**: ステータスウインドーの下に
  [攻撃][防ぐ][精神統一][投げる][盗む] のボタン列(PixelMplus テキスト。
  アイコン画像化は plan10)。クリックでキーと同じ Command を発行。
- **移動アイコンウインドー**: メッセージウインドー右に ↰↑↱ / ← ↓ → /
  R・F の小ボタン(補助入力)。
- 戦闘メッセージ: 「ガルドの こうげき! スケルトンに 12のダメージ」
  「ミス!」「スケルトンを たおした! (経験値 8)」「ガルドは レベル 4 になった!」
  「シッフは ほねのけんを ぬすんだ!」等。
- HUD の集中力バー(緑)が精神統一で伸びる様子が見えること。

## サンプルデータ(monsters.ron+level00 更新)

4種(すべて KayKit Skeletons、CC0。mage/rogue の2体を GitHub から追加取得し
CREDITS.md / assets/models/README.md に記録):

| id | モデル | 特徴 |
| --- | --- | --- |
| skel_minion | skeleton_minion | 弱。地上、視界6、幅1可、carry: bone等 |
| skel_warrior | skeleton_warrior | 中堅。逃避HPあり、carry: sword系 |
| skel_mage | skeleton_mage(追加) | 遠隔: attack_items で投擲、視界8 |
| skel_rogue | skeleton_rogue(追加) | 速い(action_unit小)、wariness高、regen付き |

- **配置の制約(重要)**: 既存 autotest(拾う〜毒)の実行エリア
  (スタート部屋、はしご・水・毒タイル周辺)にモンスターが到達しないよう、
  ドアの先/別フロアに置くか sight・action_unit を調整する。
  既存13ステップが監視なしで通り続けることが受け入れ条件。
- 投擲テスト用に throwability > 0 のアイテム(投げナイフ等)を items.ron に
  追加。props.rs は削除し、スケルトン表示はモンスター配置へ移行。

## autotest 追加ステップ(roadmap 横断ルール)

既存13ステップの後に追加(モンスターは autotest 内で必要位置に
テレポート/スポーンさせてよい。判定はすべて実システム経由):

14. `combat-hit`: 正面1マスにモンスターを配置 → Attack コマンド →
    対象HPが減る(または「ミス!」がログに出て再試行で減る)
15. `combat-kill-drop-exp`: HP1のモンスターを撃破 → carry_items が床に落ち、
    経験値ログが出て、パーティの exp が増えている
16. `levelup`: exp を閾値直前に設定→撃破→レベルと能力値が成長係数どおり上昇
17. `guard`: 防ぐ発動中の被ダメージが半減(モンスターに1回攻撃させる)
18. `throw`: 投擲でモンスターのHPが減り、アイテムが床に落ちる
19. `steal`: 盗みのうで高キャラで成功(所持品が移る)。
    用心深さ999の個体で失敗+反撃されることも確認
20. `flee`: HPを flee_hp 未満にすると距離が広がる
21. `regen`: regen_cycles 経過で復活している
22. `rest-blocked`: モンスター視認内で ZZZ 開始が拒否される

乱数(命中・徘徊)は `DEEPGRID_AUTOTEST` 時に固定シードの RNG リソースを
使い、フレーク(不安定なテスト)を避ける。通常プレイは従来どおり。

## 実装ステップ

1. データモデル+v4ローダー+ラウンドトリップテスト(19件に追加)
2. 表示+アニメ状態機械(props.rs の仕組みを monster.rs へ一般化、
   props.rs 削除。既存 DEBUG_SHOT `props` シーンは `monster` に改名)
3. 移動AI(徘徊→追跡→逃走、通行判定、液体の好み)
4. combat.rs(純関数+ユニットテスト)→ プレイヤー攻撃
5. モンスター攻撃・防ぐ・精神統一・全滅処理
6. 投擲・盗み
7. ドロップ・経験値・レベルアップ・再生・ZZZ制限
8. アクション/移動アイコンウインドー
9. サンプルデータ整備(autotest エリア回避の配置)
10. autotest ステップ 14〜22 追加+検証シーン
    (`monster`=徘徊、`combat`=戦闘メッセージ+減ったHP表示)

## 受け入れ基準

1. ビルド完走、clippy 警告なし、`cargo test` 全通過。
2. `DEEPGRID_AUTOTEST=1` が**22ステップ全PASS**(既存13+新9)で終了コード0。
3. `DEEPGRID_DEBUG_SHOT=monster|combat` が撮影でき(mtime確認)、
   既存シーン(props→monster 改名以外)も全部通る。
4. 手動確認(操作感のみ): 追跡・戦闘のテンポ、アニメーションの繋がり、
   アイコンボタンの操作感、メッセージの読みやすさ。
5. サンプルプロジェクトのデータだけで再現できる(props.rs が消えている)。

## 実装上の注意

- 上限・数値は LimitsConfig / 定数化+暫定コメント。式変更時は本文書を更新。
- AI・戦闘・再生はすべて CycleTick 駆動(フレームレート非依存)。
- アニメーション名指定で glb を引く(既存の props.rs はインデックス指定
  だったが、モンスターでは AnimationGraph 構築時に名前→インデックス解決を
  行う。解決失敗は起動時警告+idle 代替)。
- 攻撃・被弾・死亡モーション後は必ず idle/walk に戻す(状態機械を enum で)。
- 乱数は `Resource` の RNG に一元化(autotest の固定シード、将来のセーブ
  (plan10)の決定論の布石)。`rand` クレート追加可(小さい依存に留める)。
- `ClusterConfig::Single`・RenderLayers 分離・bevy_ui/egui の棲み分けは維持。
