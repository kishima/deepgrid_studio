# plan7: 魔法

## この文書について

第7実装計画書(2026-07-14、plan6/6.5 完了後の実状に合わせて詳細化済み)。
仕様源は [dandan_spec_things_editor.md](dandan_spec_things_editor.md)「Magic」と
[project.md](project.md)「魔法の仕様」。数値レンジ(MP 0〜3000 等)は参考値で
強制しない(project.md「上限値の扱い」)。開発環境の制約は [plan1.md](plan1.md)。

## ゴール

魔法の定義・習得・詠唱を一巡させる: 能力値変化/回復/復活/照明が使え、
巻物で覚え、MPを消費し、攻撃魔法は光弾で飛び、液体化(秘薬)でビンに
詰められる。

## 現状(plan6.5 完了時点の実状)

- プロジェクト形式 **v4**(`PROJECT_VERSION = 4`)。monsters.ron まで込み。
- 戦闘: combat.rs(純関数)、monster.rs(AI・PlayerAction イベント・
  投擲 `throw_item`・盗み・経験値/レベルアップ・再生)。乱数は
  `GameRng`(rng.rs、autotest 時固定シード)。
- キャラ: `CharacterState`(hp/mp/concentration/satiety/down/
  poison_remaining、食事由来の `ActiveEffect` は持続サイクル管理済み)。
  **MP はまだ何も消費しない**。`magic_knowledge` は器のみ。
- ルール: rules.rs の `RulesConfig { hunger }`(plan6.5)。
  栄養価→満腹度の係数 `satiety_per_nutrition` あり。
- アイテム: `ItemKind::Scroll` は種別として存在するが固有動作なし。
  `ItemInstance { def_id, entropy }`。データ画面に装備/はずす/食べる/
  置く/ZZZ のアクションボタンあり。
- 照明: プレイヤー追随 PointLight は movement.rs `setup_player` で
  固定値(intensity 120_000 / range 22)。
- UI: bevy_ui + PixelMplus。アクションアイコン(2×3グリッド)、
  `Command` enum は Move/Climb/ToggleDoor/Get/ToggleData/Attack/Guard/
  Concentrate/Throw/Steal。**M キーは空いている**。
- 検証: autotest **26ステップ**+DEBUG_SHOT 10シーン。
  roadmap 横断ルール: 新機能はautotestにステップ追加。

## スコープ

### やること

1. 魔法定義 `magics.ron` とプロジェクト形式 **v5**
2. 種別の実装: **能力値変化16種 + MP変更 + 復活(33/50/100%) + 照明(弱/中/強)**
   = 23種。オリジナル27種の残り4種は内訳不明のため enum を拡張可能に作り、
   判明したらこの文書に追記して追加する
3. 習得: 初期習得(characters.ron)+ 巻物(魔法知識 ≥ 難易度で「見る」→習得)
4. 詠唱: データ画面の魔法タブ(Mキー/アクションボタンから直行)、
   対象選択(味方1人/正面の敵)、MP消費、持続効果
5. 光弾(0〜2発)の飛翔演出と対モンスター魔法(対魔法力の抵抗)
6. 照明魔法(プレイヤーライトの強化、持続サイクル)
7. 秘薬: 液体化可の魔法+空容器 → 秘薬ビン(飲む/投げつける)
8. autotest 7ステップ追加+ユニットテスト+検証シーン

### やらないこと(後続planへ)

- 魔法シンボルの画像(テキスト1〜2文字で代用。画像化は plan10 の
  グラフィック差し替え)
- 効果音 → plan10
- イベント連携・秘薬のマップ配置エディター → plan8/9
- 魔法エディター → plan9(データは手書きRON)
- モンスターが魔法を使う → 将来(第2期候補)

## データモデル

`src/magic.rs`(新設):

```rust
/// 魔法種別。オリジナルは27種(dandan_spec)。不明分は判明後に追加する
/// (追加時は本文書の実装状況表を更新)。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum MagicKind {
    /// 能力値変化(StatKind 15種 + HP直接変化=栄養価枠)。
    StatChange(StatKind),
    HpChange,                       // 栄養価(HP)枠。hunger有効時は満腹度にも作用
    MpChange,
    Revive { ratio_percent: u8 },   // 33 | 50 | 100
    Light { strength: u8 },         // 1=弱, 2=中, 3=強
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MagicDef {
    pub id: String,
    pub name: String,               // 参考: 全角6文字
    #[serde(default)]
    pub description: String,        // 巻物「見る」の説明(参考: 20字×2行)
    pub mp_cost: i32,
    pub difficulty: i32,            // 必要魔法知識
    pub kind: MagicKind,
    #[serde(default)]
    pub value: i32,                 // 変更値(StatChange/HpChange/MpChange)
    #[serde(default)]
    pub duration_cycles: u64,       // 0 = 永続(即時系は無視)
    #[serde(default)]
    pub liquefiable: bool,
    #[serde(default)]
    pub projectiles: u8,            // 光弾 0〜2。>0 なら敵対象の攻撃魔法
    #[serde(default)]
    pub symbol: String,             // 表示用1〜2文字
}
```

- `MagicCatalog` リソース(ItemCatalog と同型。id重複拒否、iter あり)。
- project.ron v5: `magics: "magics.ron"`(serde default、v4以前は魔法なし)。
- 習得状態: `CharacterState.learned: Vec<String>`(magic id、保存対象)。
  初期習得は `Character.magics: Vec<String>`(serde default)から
  `build_party` で流し込む(plan5 の items と同じパターン)。
- 巻物: `ItemDef.teaches: Option<String>`(serde default)。

## 挙動仕様

### 習得(巻物)

- データ画面で巻物スロットを選択すると **[見る]** ボタンが出る(新規)。
  押すと description を メッセージへ表示し、`teaches` があれば:
  - 選択キャラの魔法知識(effective)≥ difficulty → 習得+巻物消滅
    「メリナは 『ヒール』を おぼえた!」
  - 不足 → 「むずかしくて 理解できない」(巻物は残る)
  - 習得済み → 「すでに おぼえている」(巻物は残る)

### 詠唱

- 入口: `M` キー / アクションアイコン「魔法」(取るの隣に追加、2×3→
  グリッド拡張可)→ データ画面を魔法タブで開く。
- 魔法タブ: 選択キャラの習得魔法一覧(シンボル+名前+MP)。選択で
  詳細(説明/難易度/持続)+ **[唱える]**、liquefiable なら **[液体化]**。
- 対象: kind から自動決定。
  - `projectiles > 0` → 正面の敵(タブを閉じてメイン画面で発射)
  - `Revive` → 気絶メンバー選択(いなければ「たおれている仲間はいない」)
  - その他 → 味方1人(パーティカードのクリックで選択、既定=詠唱者)
- 消費と判定: MP < mp_cost なら「MPが たりない」。詠唱で MP -= mp_cost。
  魔法知識 < difficulty の魔法は一覧でグレーアウト(唱えられない)。
- 効果:
  - StatChange: `ActiveEffect { stat, delta: value, remaining }` として適用
    (食事効果と同じ枠組み。duration 0 = 永続)。同一 magic id の重ねがけは
    持続リセット(加算しない。暫定)。
  - HpChange: HP += value(0〜max_hp にクランプ)。hunger 有効時は
    満腹度 += value × `rules.hunger.satiety_per_nutrition`(plan6.5 の既定)。
  - MpChange: MP += value(0〜max_mp)。
  - Revive: down 解除、HP = max_hp × ratio/100(最低1)。
  - Light: 下記。

### 攻撃魔法(光弾)

- 発射: プレイヤー正面方向へ、`throw_item` と同じ直線走査(壁・閉ドアで
  停止、最初のモンスターに命中)。射程は 8 マス(暫定定数)。
- 演出: 発光球(emissive の小スフィア)を光弾数ぶん 0.15秒間隔で連射。
  飛翔は実時間補間(サイクル非依存の見た目のみ)。lavapipe で重い場合は
  1発に簡略化してよい(定数化)。
- ダメージ: `基本 = |value|`、抵抗 `有効率 = clamp(100 − 対魔法力/10, 5, 100)%`
  を光弾ごとに判定(暫定式。combat.rs に純関数で置きユニットテスト)。
  被弾モンスターは Hit アニメ+攻撃者の方を向く(plan6 の facing_toward)。
  撃破処理は kill_monster を流用。

### 照明魔法

- `setup_player` の固定値を「基準値」定数に切り出し、`LightBoost` リソース
  (倍率+残りサイクル)を新設。毎サイクル減衰し、切れたら基準値へ戻す。
- 倍率(暫定): 弱 ×1.5 / 中 ×2.5 / 強 ×4.0(intensity と range の両方)。
- 重ねがけは強い方を採用し持続をリセット。
- HUD: 照明中はメッセージ「あたりが あかるくなった」+切れた時
  「あかりが きえていく…」。

### 秘薬(液体化)

- 魔法タブの **[液体化]**: 詠唱者の手/ポーチ/リュックに `EmptyContainer` が
  あれば MP を消費して `potion_of: Some(magic_id)` のビンに変換
  (`ItemInstance` に `#[serde(default)] pub potion_of: Option<String>` を追加)。
  なければ「からのビンを もっていない」。
- 表示名は「〜のビン」(データ画面詳細に魔法名)。
- **[飲む]**(potion_of ありのスロット選択時): 詠唱と同じ効果を自分に適用
  (MP消費なし — コストは生成時に払済み)。ビンは空容器に戻る。
- **投げつける**: 既存の Throw で投げ、モンスター命中時に魔法効果を適用
  (StatChange/HpChange の負値のみ意味がある。対魔法力の抵抗判定あり)。
  ビンは割れて消滅(床に落ちない)。
- Light/Revive の秘薬は不可(liquefiable: false をデータ規約とし、
  ローダーで警告)。

## サンプルデータ(v5)

magics.ron に8種(名前は雰囲気優先で自由):

| id | kind | 例 |
| --- | --- | --- |
| heal | HpChange(+30) | 液体化可。ヒール |
| firebolt | HpChange(−25)? → **敵対象** projectiles:2 | ファイアボルト |
| shield | StatChange(Defense,+10) 持続300 | シールド |
| haste | StatChange(Agility,+15) 持続300 | ヘイスト |
| mind | MpChange(+20) | マインド |
| revive50 | Revive{50} | リザレク |
| light2 | Light{2} 持続600 | ランタン |
| venom | StatChange(PoisonResist,−50)? → 敵用・液体化可(投擲毒) | ベノム |

- ※攻撃魔法のダメージは `|value|` を使うため firebolt は value:−25 で表す
  (符号規約: 負=対象を害する。ローダー検証はしない)。
- 巻物: scroll_heal(teaches: heal、難易度低)、scroll_fire(teaches:
  firebolt、難易度高め=メリナのみ可)を items.ron に追加し level00 に配置。
- メリナ(mage): 初期習得 [firebolt, light2]、magic_knowledge を巻物検証に
  合う値へ調整。ソロン(priest 役): [heal, revive50]。
- 空容器 `bottle_empty` を items.ron に追加(既存 water_bottle とは別)。

## autotest 追加ステップ(27〜33)

- 27 `learn-scroll`: 巻物で習得成功(learned に追加・巻物消滅)。
  知識不足キャラでは失敗し巻物が残る
- 28 `cast-buff`: shield 詠唱 → MP減・防御が effective で上昇 →
  持続サイクル経過で元に戻る
- 29 `cast-attack`: 正面のモンスターに firebolt → HP減。
  対魔法力999のテスト個体には抵抗される(固定シードで決定的に)
- 30 `cast-revive`: 気絶させた味方に revive50 → down解除・HP=50%
- 31 `mp-gate`: MP を 0 にして詠唱 → 拒否メッセージ、MP・効果とも変化なし
- 32 `light`: light2 詠唱 → PointLight の intensity が基準×2.5、
  持続切れで基準値に戻る(Light コンポーネントを直接アサート)
- 33 `potion`: heal を液体化 → potion_of 付きビン生成 → HPを減らして飲む →
  回復+空容器に戻る

## 実装ステップ

1. magic.rs+v5 ローダー+ラウンドトリップ/習得判定のユニットテスト
2. 習得(初期+巻物の[見る])
3. 詠唱基盤(魔法タブUI、MP、対象、StatChange/HpChange/MpChange/Revive)
4. 光弾と攻撃魔法(抵抗式は combat.rs、境界値ユニットテスト)
5. 照明魔法(LightBoost)
6. 秘薬(液体化/飲む/投げつける)
7. サンプルデータ+autotest 27〜33+検証シーン
   `magic`(光弾または着弾直後のログ)、`light`(照明中の明るい画面)、
   `potion`(秘薬ビンのあるデータ画面)

## 受け入れ基準

1. ビルド完走、clippy 警告なし、`cargo test` 全通過。
2. `DEEPGRID_AUTOTEST=1` が **33ステップ全PASS**・終了コード0。
3. `DEEPGRID_DEBUG_SHOT=magic|light|potion` が撮影でき(mtime確認)、
   既存10シーンも全部通る。
4. 手動確認(操作感のみ): 魔法タブの操作感、光弾の見た目、照明の変化。
5. サンプルプロジェクトのデータだけで再現できる。

## 実装上の注意

- 実装済み/未実装の種別を本文書の表で管理(不明4種の枠を含む)。
- 効果適用は既存の `ActiveEffect` 枠に統一(装備効果と混ぜない、
  基礎値を書き換えない)。セーブ(plan10)を見据え、`learned` と
  `potion_of` は serde 互換を保つ。
- 対魔法力・光弾数・射程・倍率など暫定値はすべて定数化+コメント。
  将来 RulesConfig への移住候補(plan9)。
- 乱数(抵抗判定)は GameRng 経由(autotest の決定性)。
- MagicKind の未知バリアントは「ロードエラーで止まる」でよい
  (エディター登場(plan9)までデータは手書きのみ。将来種別を追加するときは
  enum に足して本文書の表を更新する、が正規の手順)。
- `ClusterConfig::Single`・RenderLayers・bevy_ui/egui の棲み分けは維持。
