# plan4: キャラクターとメイン画面UI

## この文書について

DeepGrid Studio の第4実装計画書。前提知識は [project.md](project.md)(仕様)、
[roadmap.md](roadmap.md)(全体計画)、plan1〜3(実装済み)。
開発環境の制約(docker 必須、`CARGO_TARGET_DIR`、ホストで `cargo run` 不可)は
[plan1.md](plan1.md)「開発環境の前提」を必ず先に読むこと。
素材の記録は CLAUDE.md「素材の記録ルール」に従う(本plan用の素材は
導入・記録済み。下記「用意済みの素材」)。

## ゴール

「誰がダンジョンを歩いているのか」を画面に出す。
キャラクターのデータモデルとパーティ(4人)を導入し、メイン画面に
**ステータスウインドー**(ポートレート+HP/MP/集中力バー)と
**メッセージウインドー**を載せる。あわせてリアルタイム処理の土台となる
**サイクル時間システム**を入れ、最初のダメージ源として**落下ダメージ**を実装する。

## 用意済みの素材(この plan で使うこと)

| ファイル | 内容 |
| --- | --- |
| assets/models/party/knight.glb | 戦士(KayKit Adventurers、CC0) |
| assets/models/party/mage.glb | 魔法使い(同上) |
| assets/models/party/rogue.glb | 盗賊(同上) |
| assets/models/party/rogue_hooded.glb | 僧侶役(フード姿。同上) |
| assets/models/party/barbarian.glb | 蛮族(同上) |
| assets/fonts/PixelMplus12-Regular.ttf / -Bold.ttf | 日本語ピクセルフォント(M+ライセンス) |

- Adventurers はスケルトンと同じリグで **76アニメーション**入り
  (`Idle` はインデックス 36。全キャラ同一。src/props.rs の仕組みがそのまま使える)。
- 職業はオリジナル(だんだんダンジョン)には無い概念で、キャラの
  **プロフィール(経歴)+見た目モデルの対応**として扱う。Wizardry風の
  雰囲気づけであり、ゲームルール上の職業システムは作らない。

## スコープ

### やること

1. キャラクターのデータモデル(能力値・成長タイプ・プロフィール)
2. プロジェクト形式の拡張: `characters.ron`(登録キャラ)と `party`(編成)
3. サンプルプロジェクトに5キャラ+4人パーティのデータを用意
4. メイン画面UI(bevy_ui): ステータスウインドー+メッセージウインドー
5. ポートレート表示(パーティキャラの3Dバストをレンダーターゲットに描画)
6. サイクル時間システムの基盤(`GameClock`)と集中力の自然回復
7. 落下ダメージ(plan2 から持ち越し)
8. characters.ron ラウンドトリップ等のユニットテスト
9. 検証シーンのHUD対応

### やらないこと(後続planへ)

- アイテム・装備・運搬力の実効果 → plan5
- データ画面(装備・持ち物・ZZZ・詳細ステータス) → plan5
- 戦闘、アクションアイコンウインドー、移動アイコンウインドー → plan6
- 魔法、MPの消費 → plan7
- レベルアップ・経験値(成長タイプはデータとして持つだけ) → plan6以降
- HP0時の死亡処理・復活(plan4では「気絶」表示のみ) → plan7/8
- キャラクターエディター → plan9
- 液体ダメージ(耐性パラメーターの実効果) → plan5

## データモデル

`src/character.rs`(新設):

```rust
/// 能力値(project.md「キャラクターの仕様」+ dandan_spec_things_editor.md)。
/// plan4 で実際に参照するのは hp/mp/concentration まわりのみだが、器は全項目そろえる。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Stats {
    pub level: u32,          // オリジナルは初期0〜99/最大255だが参考値(制限しない)
    pub max_hp: i32,
    pub max_mp: i32,
    pub attack: i32,
    pub defense: i32,
    pub agility: i32,        // すばやさ
    pub throwing: i32,       // 遠投力
    pub carrying: i32,       // 運搬力
    pub lung_capacity: i32,  // 肺活量
    pub heat_resist: i32,    // 耐熱力
    pub poison_resist: i32,  // 耐毒性
    pub magic_knowledge: i32,// 魔法知識
    pub concentration: i32,  // 集中力(最大値)
    pub appraisal: i32,      // 鑑定力
    pub stealing: i32,       // 盗みのうで
    pub bite: i32,           // 歯の強さ
}

impl Stats {
    /// 総合レベル: 全能力パラメーターの平均(dandan_spec: 強さの目安)。
    /// 保存はせず導出する。表示は plan5 のデータ画面から。
    pub fn overall_level(&self) -> i32 { /* 平均 */ }
}

/// 成長タイプ(dandan_spec: 平均型/早期開花型/大器晩成型/天才型/才能なし)。
/// レベルアップ時の伸び方に影響する。plan4 ではデータとして持つだけ。
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum GrowthType { Average, EarlyBloomer, LateBloomer, Genius, Talentless }

/// プロフィール(dandan_spec_things_editor.md「名前・プロフィール項目」)。
/// 感情移入のための項目群で、plan4 でゲームロジックには使わない。
/// オリジナルの文字数・数値レンジ(全角6文字等)はPC98由来の参考値であり
/// **強制しない**(project.md「上限値の扱い」)。UIに収まらない場合は省略表示。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Character {
    pub id: String,            // プロジェクト内で一意("knight" 等)
    pub first_name: String,    // ゲーム中に表示される名前
    pub last_name: String,     // 名字。ゲーム中は非表示
    pub gender: String,
    pub height_cm: f32,
    pub weight_kg: f32,
    pub birth_date: String,    // "YYYY-MM-DD"
    pub age: u32,
    pub likes: String,         // 好きなもの
    pub dislikes: String,      // 嫌いなもの
    pub background: String,    // 経歴(複数行可。Wizardry風の職業ラベルはここ)
    pub growth: GrowthType,
    pub stats: Stats,
    pub model: String,         // 見た目: プロジェクト相対 or assets相対のglbパス
}

/// プレイ中の可変状態(HPなど)。定義(Character)と分離する。
pub struct CharacterState {
    pub hp: i32,
    pub mp: i32,
    pub concentration: i32,
    pub down: bool,            // HP0 で気絶
}
```

プロジェクト形式の拡張(plan3 の project.ron に追記):

```ron
(
    name: "Sample Dungeon",
    version: 2,                       // 1→2 に上げる(characters/party 追加)
    limits: ( ... ),
    levels: ["levels/level00.ron"],
    characters: "characters.ron",     // 登録キャラ一覧(最大 limits.max_characters)
    party: ["knight", "mage", "rogue_hooded", "rogue"],  // id 参照、最大 party_size
)
```

- `characters.ron` は `Vec<Character>`。sample には5キャラを手書きする
  (名前・経歴は雰囲気重視で自由に。例: ガルド/戦士、メリナ/魔法使い、
  ソロン/僧侶、シッフ/盗賊、バルグ/蛮族。プロフィール項目も全部埋めて
  データ例として機能させる)。
- **version 1 のプロジェクトも読めること**: characters/party が無い場合は
  空パーティ(UI はステータスウインドー非表示)で起動し、警告ログを出す。
- バリデーション: party の id が characters に存在、party 人数 ≤
  `limits.party_size`、キャラ数 ≤ `limits.max_characters` のみ。
  プロフィールの文字数・数値範囲は検証しない(PC98由来の参考値のため。
  project.md「上限値の扱い」)。初期レベルのオリジナル値 0〜99 も参考値と
  し、型は u32 のまま制限しない。

## メイン画面UI

**bevy_ui で実装する(egui は使わない)。** 理由: bevy_egui はウィンドウ直描きの
ため Bevy の `Screenshot` に写らず(plan3 Step 6 の知見)、プレイ画面は
スクリーンショット検証が必須のため。egui はエディター専用のままとする。
Cargo feature に `bevy_ui` `bevy_text` `default_font` を追加する。

日本語テキストはすべて `PixelMplus12-Regular.ttf`(強調は -Bold)を使う。

```
+----------------------------------------------------------+
|                                            | ステータス   |
|                                            | ウインドー   |
|              3Dビュー(既存)               | ┌─────────┐ |
|                                            | │P1 [顔] 名前│|
|                                            | │HP ███▁ 青 │|
|                                            | │MP ██▁▁ 赤 │|
|                                            | │集 ████ 緑 │|
|                                            | └─(×4人)──┘ |
+--------------------------------------------+-------------+
| メッセージウインドー(最新4行表示、上に古い行)            |
+----------------------------------------------------------+
```

- **ステータスウインドー**(右サイドバー、幅 ~220px): パーティ4人分のカード。
  ポートレート(下記)+名前+3本のバー。バーの色はオリジナル準拠で
  **HP=青、MP=赤、集中力=緑**(project.md「メイン画面」)。値が減ると
  バーが縮む。HP0 のカードは灰色化し「気絶」と表示。
- **メッセージウインドー**(下部、高さ ~96px): リングバッファ(256行)に
  イベントを積み、最新4行を表示。plan4 で流すイベント:
  「そちらには進めない」「ドア1が開いた/閉じた」「はしごを上った/下りた」
  「N フロア落下した! ○○は Xのダメージ」。書き込みは `MessageLog`
  リソース(`pub fn push(&mut self, text: String)`)経由に統一。
- ウィンドウリサイズに追従すること(固定解像度前提にしない。
  3Dビューのカメラビューポートは全画面のままでよい: UI が上に載る形)。

## ポートレート

パーティ各キャラの glb を**バストアップの3Dポートレート**として表示する:

- キャラごとに 128×128 の `Image` レンダーターゲット+専用カメラを作り、
  頭部〜胸あたりを正面から映す(モデルは `RenderLayers` でメインカメラから
  隔離した専用レイヤーに置く。ライトも専用に1灯)。
- モデルには `Idle` アニメーション(インデックス36)を再生する
  (src/props.rs の `attach_prop_animations` を汎用化して流用)。
- ステータスウインドーの `ImageNode` にそのハンドルを渡す。
- **lavapipe での性能に注意**: 4キャラ×128×128 が重い場合は、数フレーム
  描画後にポートレートカメラを `is_active = false` にして静止画化する
  フォールバックを入れる(その場合アニメは止まってよい)。判断基準は
  `props` シーン相当の描画で体感がもたつくかどうかでよい。

これにより静止画ファイルの生成は不要(PNG ポートレートは作らない)。
オリジナルの「顔のグラフィック」(グラフィックエディターで自作した肖像画)に
相当する機能で、ユーザー画像による差し替えは plan10(グラフィック差し替え機構)で扱う。

## サイクル時間システム

`src/clock.rs`(新設):

```rust
/// リアルタイム→サイクル変換(project.md「リアルタイム処理」)。
/// 1サイクル = 0.1秒(定数 CYCLE_SECS)。ゲーム内の時間コストはすべて
/// サイクル単位で表す。
#[derive(Resource)]
pub struct GameClock {
    pub cycle: u64,        // 起動からの累計サイクル
    accum: f32,
}
```

- 毎フレーム `Time` から加算し、サイクル境界をまたいだ回数だけ
  「サイクルイベント」を発火する(1フレームで複数サイクル進むことがある)。
- plan4 でサイクルに載せる処理:
  - **集中力の自然回復**: 1サイクルごとに全員 +1(最大値まで)。
  - (器だけ)後続planの毒・炎ダメージ、モンスターの行動もここに載る想定で、
    `on_cycle` 的なシステム分離をしておく。
- 移動や落下のアニメーション時間は従来どおり実秒で管理してよい
  (行動のサイクルコスト消費は戦闘を入れる plan6 で扱う)。

## 落下ダメージ

- 落下完了時、落下フロア数 n(1以上)に対し **ダメージ = 10 × n²** を
  パーティ全員に与える(すばやさ等による軽減は plan6 の計算システムで検討。
  この式は暫定であることをコードコメントに明記)。
- HP は 0 未満にしない。0 になったキャラは `down = true`(気絶)。
  全員気絶してもゲームオーバー処理はまだ無い(メッセージのみ)。
- メッセージ例: 「2フロア落下した!」「ガルドは 40 のダメージ!」

## 実装ステップ

1. **データモデル**: character.rs、project.rs の拡張(version 2)、
   sample の characters.ron+party 追記。ラウンドトリップと
   バリデーションのユニットテスト。
2. **GameClock**: clock.rs とサイクルイベント、集中力回復。
   サイクル変換のユニットテスト(累積誤差が出ないこと)。
3. **MessageLog と HUD**: bevy_ui でステータス/メッセージウインドー。
   フォント読込。パーティは characters.ron から `CharacterState` を初期化。
4. **ポートレート**: レンダーターゲット+専用レイヤー+Idle再生。
5. **落下ダメージ**: 落下完了フックからダメージ適用+メッセージ+バー反映。
6. **検証シーン**: 既存シーンに HUD が写るようになるので、
   受け入れ基準の画像確認項目を満たすことを確認。
   `fall` シーンは落下後に HP バーが減った状態+ダメージメッセージが写ること。

## 受け入れ基準

1. `./docker/deepgrid-build.sh` 完走、ホスト `cargo clippy` 警告なし、
   `cargo test` 全通過。
2. `DEEPGRID_DEBUG_SHOT=1`: 3Dビューの右にステータスウインドー(4人分の
   ポートレート+青/赤/緑バー+日本語名)、下にメッセージウインドーが写る。
   ポートレートに各キャラの顔(Knight/Mage/Rogue_Hooded/Rogue)が写っている。
3. `DEEPGRID_DEBUG_SHOT=fall`: 落下後、HPバーが減り、落下・ダメージの
   日本語メッセージがメッセージウインドーに写っている。
4. `DEEPGRID_DEBUG_SHOT=ladder|door|props|editor` が引き続き通る。
5. 手動確認(人間が実施): 通常プレイでバーとメッセージが動き、
   ウィンドウをリサイズしてもUIが破綻しない。
6. すべてのスクリーンショットで mtime が当該実行時刻であること。

## 実装上の注意

- 上限(party_size=4、max_characters=20、level≤255)は LimitsConfig 経由。
  ハードコード禁止。
- UI の文言は日本語。文字化けする場合はフォント読込(PixelMplus)を疑う
  (bevy の default_font は日本語グリフを持たない)。
- `CharacterState`(可変)と `Character`(定義)を混ぜない。セーブ対象に
  なるのは前者(セーブ実装は plan10)。
- ポートレートの RenderLayers がメインの3Dビューやポイントライトと
  干渉しないこと(専用レイヤーにライトを置き、メインレイヤーに漏らさない)。
- `ClusterConfig::Single` は引き続きメインカメラに必要(plan1 の知見)。
  ポートレートカメラにも同様のライト矩形が出る場合は同じ対策を入れる。
- 落下ダメージ式(10×n²)は暫定。式を変える場合はこの文書を更新する。
