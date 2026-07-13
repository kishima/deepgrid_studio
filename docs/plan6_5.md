# plan6.5: 空腹度(満腹度)システムとルール設定の導入

> **注意**: plan6(モンスターと戦闘)実装中に書かれた差し込みplan。
> 着手時に plan6 完了後の実状(プロジェクト形式のバージョン、autotest の
> ステップ数、CharacterState の形)と照合し、差分があれば本文書を更新する。

## この文書について

DeepGrid Studio 独自拡張の第1弾(オリジナルのだんだんダンジョンには無い、
ダンジョンマスター由来の要素)。ユーザー決定(2026-07-13):
**「栄養価=HP回復」というオリジナル仕様は壊さず、満腹度を層として追加する。**
開発環境の制約は [plan1.md](plan1.md)、無人テストの方針は roadmap 横断ルール。

## ゴール

1. 満腹度(satiety)の導入: 時間で減り、食事で回復し、尽きると飢餓。
2. **`RulesConfig`(プロジェクト単位のゲームルール設定)の枠を新設**し、
   空腹度を最初の住人にする。作るゲームごとに ON/OFF・数値調整が可能。

## スコープ

### やること

1. project.ron に `rules` セクション(RulesConfig、serde default で後方互換)
2. CharacterState への満腹度追加とサイクル駆動の減衰・飢餓
3. 食事との接続(満腹度回復)、ZZZ休息との接続(飢餓中は回復しない)
4. HUD 4本目バー(オレンジ)+警告メッセージ
5. autotest ステップ追加+ユニットテスト

### やらないこと

- ルール設定のエディターUI → plan9(プロジェクト設定エディターに合流)
- 既存の暫定定数(落下ダメージ・液体ダメージ等)の RulesConfig への移住
  → 器だけ作る。移住は plan9 のエディター化と同時でよい
- 水分(喉の渇き)の分離 → 第2期候補(必要なら roadmap に記録)
- モンスターの空腹 → 対象外

## データモデル

`src/config.rs`(または新設 `src/rules.rs`)に追加:

```rust
/// プロジェクト単位のゲームルール(project.ron の `rules`)。全フィールド
/// serde(default) で、無指定の旧プロジェクトは「空腹度なし」で動く。
#[derive(Resource, Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct RulesConfig {
    pub hunger: HungerRules,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct HungerRules {
    pub enabled: bool,               // 既定 false(旧プロジェクト互換)
    pub satiety_max: i32,            // 1000
    pub drain_interval_cycles: u64,  // 10(=1秒に1減。1000で約16分)
    pub starvation_damage: i32,      // 1(/サイクル、満腹度0のとき)
    pub satiety_per_nutrition: i32,  // 10(食事: 満腹度 += 栄養価×これ)
    pub warn_ratio: f32,             // 0.25(これ未満で警告)
}
```

- **sample プロジェクトは enabled: true** で全数値を明示的に書く
  (データ例として機能させる)。
- `CharacterState` に `satiety: i32` を追加。初期値 = satiety_max。
  ZZZ・気絶からの復帰等で満腹度は回復しない(食事のみ)。

## 挙動仕様

すべて CycleTick 駆動(hazard.rs / clock.rs の既存パターンに従う):

1. **減衰**: `drain_interval_cycles` ごとに全員 satiety -= 1(0で下げ止め)。
   休息(ZZZ)中も減る。気絶中も減る。
2. **食事**: 既存の `eat` 成功時に `satiety += 栄養価 × satiety_per_nutrition`
   (satiety_max で頭打ち)。**HP回復(既存挙動)はそのまま**。
   栄養価が負のアイテムは満腹度も減らす(対称に)。
3. **飢餓(satiety == 0)**:
   - `starvation_damage` /サイクルのHPダメージ(HP0で気絶、既存と同じ扱い)
   - 集中力の自然回復が止まる(clock.rs の回復処理でガード)
   - ZZZ休息のHP/MP回復が無効(休息自体は続けられるが回復しない)
4. **警告**: satiety が `satiety_max × warn_ratio` を下回ったら
   「おなかがすいた…」、0 になったら「うえじにしそうだ!」を
   メッセージウインドーへ(hazard と同様のスロットリングで連投抑止)。
5. **enabled: false**: 上記すべて無効。satiety は満タンのまま動かない
   (HUDのバーも非表示)。

## UI

- HUD ステータスカードに**4本目の細いバー(オレンジ)**を追加
  (HP青/MP赤/集中緑の下)。hunger 無効のプロジェクトでは行ごと出さない。
- データ画面のステータス欄に「満腹度 850/1000」を追加。
- 飢餓中のキャラのカードに「飢餓」表示(気絶の表示と同じ枠)。

## autotest 追加ステップ

plan6 完了時点の末尾(想定22)に続けて追加。満腹度の操作は
`member.state.satiety` への直接代入でよい(減衰・飢餓の判定が実システムを
通ることが本質):

- `hunger-drain`: 数十サイクル経過で satiety が仕様どおり減っている
- `hunger-eat`: 食事で satiety が 栄養価×係数 増える(HP回復も従来どおり)
- `hunger-starve`: satiety=0 にして数サイクル → HPが減り、警告ログがあり、
  集中力が回復していない
- `hunger-rest`: 飢餓中に ZZZ → HPが回復しない
- `hunger-off`: RulesConfig を無効に差し替えたセッション…は起動を跨ぐため、
  代わりに**ユニットテスト**で「enabled:false の HungerRules では減衰・飢餓
  関数が no-op」を検証する(rules 無指定の旧 project.ron が読めることも
  ラウンドトリップテストに追加)

## 実装ステップ

1. RulesConfig(serde default、ラウンドトリップ+後方互換のユニットテスト)
2. satiety 追加+減衰+飢餓(サイクル駆動)
3. eat / ZZZ / 集中力回復との接続
4. HUD バー+データ画面表示+警告メッセージ
5. sample プロジェクトの rules 記述(enabled: true)
6. autotest 4ステップ+ユニットテスト

## 受け入れ基準

1. ビルド完走、clippy 警告なし、`cargo test` 全通過(後方互換テスト含む)。
2. `DEEPGRID_AUTOTEST=1` が既存全ステップ+hunger 4ステップで PASS、
   終了コード0。
3. `DEEPGRID_DEBUG_SHOT=1` で4本目のオレンジバーが写っている
   (hunger 有効の sample にて。mtime 確認)。
4. 手動確認(操作感のみ): バーの減りの体感速度、警告メッセージの頻度が
   鬱陶しくないか。※数値調整はすべて project.ron で完結すること。

## 実装上の注意

- 数値のハードコード禁止 — すべて RulesConfig 経由(このplanの主目的の
  半分は「ルールをデータにする」枠の確立)。
- 飢餓ダメージは hazard.rs の毒と同様のパターンで実装し、メッセージの
  スロットリングも合わせる。
- 既存 autotest の水・毒ステップは HP 変動を監視している —
  減衰間隔(既定10サイクル)では干渉しないが、hunger の飢餓ダメージが
  それらのステップ中に発生しない満腹度初期値であることを確認する
  (初期=満タンなので通常は問題ない)。
- plan7(魔法)には栄養価(HP)系の魔法種別がある — 満腹度への効果は
  「栄養価アイテムと同じ係数で満腹度にも作用する」を既定とし、plan7 側で
  本文書を参照すること。
