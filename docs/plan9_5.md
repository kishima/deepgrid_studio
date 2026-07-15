# plan9.5: 3Dエディットモード

## この文書について

第9.5実装計画書(2026-07-15、plan9 完了後の実状に合わせて作成)。
plan9 で「次期送り」とした 3Dエディットモードの実装。仕様源は
[dandan_spec_mapeditor.md](dandan_spec_mapeditor.md)(3Dエディットモード/
その他の処理)と [plan9.md](plan9.md)「3Dエディットモード」節。
開発環境の制約は [plan1.md](plan1.md)。

## ゴール

オリジナル同様、**ダンジョンの中を歩きながらマップを作れる**ようにする。
2D俯瞰と3D一人称を切り替え、正面のセルにパレットのパーツを設置/消去する。

## 現状(plan9 完了時点の実状)

- エディター(editor::run)は **Camera2d+EguiPlugin のみ**の App。
  3D描画・カメラ・メッシュ系は一切登録されていない。
- 再利用できる部品(すべて実装済み):
  - `render`: `spawn_level_mesh`(LevelData→メッシュ一括生成)、
    `TileDirty`+`rebuild_dirty_tiles`(セル+隣接の部分再構築)、`Palette`
  - プレイ側の `MoveMode::Free`(自由飛行: 落下なし・R/Fで任意フロア昇降)
    — ただし movement.rs はパーティ・ドア・データ画面等と密結合
  - エディターの `EditorState`(EditCmd/EditOp、Undo/Redo・dirty 一元化)と
    `PlaceLayer`(ブロック/アイテム/モンスター/トリガー)、egui パレット
  - アイテムの床表示(モデル+汎用ジェム)は floor_items.rs にあるが
    プレイ用リソース(Party 等)前提
- 検証: autotest 43、DEBUG_SHOT はプレイ17+エディター(egui)7シーン。
  エディターの egui は EguiRenderToImage(shot.rs)で撮影、
  **Bevy の Screenshot に egui は写らない**(plan3 の既知事項)。

## スコープ

### やること

1. マップタブの **[2D/3D] トグル**と、エディター App への3D描画の追加
2. **エディターウォーク**(グリッド移動の軽量実装。壁は通れない・
   落下しない・R/Fで任意フロア昇降)
3. **正面セルへの設置/消去**(左クリック=選択パーツ設置、右クリック=
   Empty化。全 PlaceLayer 対応、既存 EditCmd 経由で Undo/Redo 有効)
4. 3D表示への**編集の即時反映**(TileDirty)と、アイテム/モンスター/
   トリガーの3Dマーカー表示(隠しワープ含め編集中は常に可視)
5. 座標・向きの常時表示、フロア/レベル切替の3D追随
6. 撮影シーン `editor-3d`+ユニットテスト

### やらないこと

- 3D内でのイベントパラメーター編集(設置→イベントタブへ、は従来どおり)
- 壁抜け移動(2Dで消す方が速い。オリジナルの「自由移動」は昇降自由までとする)
- ワープ・しかけ床の「作動トグル」(エディターにイベント系は登録しない
  ため常時OFF。UIだけ植えるのもやめる — 実体のないトグルを置かない)
- プレイモードとのシームレス切替(第2期候補のまま)

## 設計

### App 構成(editor::run の拡張)

- 常時登録に変更するもの: `render` の Palette 初期化、TileDirty イベント、
  `rebuild_dirty_tiles`。3Dカメラ+メッシュは **3Dモード突入時にスポーン**、
  2Dへ戻る時に一括 despawn(`Edit3dScoped` マーカー)。
- egui は継続使用(上部バー・左パレット・警告パネルはそのまま)。中央は
  egui のパネルを置かず、背後の3Dビューを見せる(bevy_egui は背景が
  透過するので、CentralPanel を **描かない**だけでよい)。
- Camera2d は3Dモード中は `is_active = false`(2D描画コードは温存)。
- ポートレート・HUD・hazard・monster AI 等プレイ専用システムは**登録しない**
  (plan9 の注意事項を維持)。

### エディターウォーク(src/editor/walk.rs 新設)

movement.rs は流用しない(プレイ結合が強い)。軽量に再実装する:

```rust
pub struct EditWalk { pub pos: GridPos, pub facing: Facing, /* 補間用 */ }
```

- キー: WASD(前後+ストレイフ)/QE・←→(回転)/↑↓(前後)/
  R・F(フロア上下 — はしご不要、範囲内クランプ)。
  egui がキーボードフォーカスを持つ間(テキスト入力中)は無視する
  (`egui_ctx.wants_keyboard_input()` でガード)。
- 通行判定: **編集中の LevelData**(EditorState が正)に対して
  「Wall だけが移動を塞ぐ」(ドア・ホロスコープ・液体・穴はすべて通過可。
  穴・支持なしでも落ちない)。
- 見た目はプレイと同じ 0.25秒スムーズ補間+90度回転(walk.rs 内に
  最小限の Segment 相当を持つ。ease は smoothstep)。
- カメラ高さ EYE_HEIGHT、FOV もプレイと同値(定数を re-use)。

### 設置/消去

- マウス左クリック: 現在の `PlaceLayer` の選択パーツを**正面1マスのセル**へ
  設置(2Dと同じ EditCmd を発行 → LevelData 更新 → TileDirty)。
  - Block レイヤ: ブロック置換(2Dと同一の意味論。トリガーブロックは
    イベント雛形の自動生成も2Dと同じ経路で発動)
  - アイテム/モンスター: 配置追加(同一マス重複は2Dと同じ規則)
  - トリガー: ブロック設置+雛形生成
- 右クリック: 正面セルの消去(Block→Empty、配置レイヤ→そのマスの配置を除去。
  これも既存の消去 EditCmd と同じ)。
- クリックが egui のUI上だった場合は無視(`wants_pointer_input()` ガード)。
- 自分の足元セルは対象外(正面のみ。オリジナルの操作感に合わせる)。

### 3D表示

- 地形: `spawn_level_mesh` をそのまま使用。編集は TileDirty で部分反映。
  フロア切替(egui)は「現在フロアの1層のみ表示」ではなく**全フロア表示**
  (プレイと同じ見え方)とし、ウォークのいるフロアが基準。
  ※2Dのフロア選択と EditWalk.pos.floor は連動させる。
- 配置マーカー(`Edit3dScoped`):
  - アイテム: モデルがあれば glb、なければ種別色のジェム
    (floor_items の生成関数を Party 非依存の形に切り出して共用)
  - モンスター: モデルを Idle 静止で表示(アニメ再生はしない — 負荷と
    実装量の節約。スケールはプレイと同値)
  - トリガー/隠しワープ/書ける壁: 種別色の半透明マーカー板+記号
    (2Dパレットの色と揃える)。**隠し属性も常に表示**
- 配置の追加/除去時は該当マーカーだけ差し替え(全再構築しない)。

### HUD(egui)

- 上部バーに [2D/3D] トグル、3D中は「(x, y, F floor) 向き N/E/S/W」を
  常時表示(その他の処理「座標表示」— 常時ONで開始し、トグルは設けない)。
- パレット・レベル/フロア切替・Save All・Undo/Redo は2Dと完全共通
  (3D中の Undo/Redo も TileDirty/マーカー差し替えで反映されること)。

### 撮影・テスト

- `DEEPGRID_DEBUG_SHOT=editor-3d`: 3Dモードで数歩進んだ位置から
  **Bevy Screenshot** で撮影(3Dビューが写る。egui パネルは写らない —
  既知事項として本文書に明記。egui 側は既存 editor-map シーンが担保)。
- ユニットテスト: walk の通行判定(壁ブロック/フロアクランプ/
  ドア・穴は通過可)、正面セル計算、設置/消去 EditCmd が 2D 経路と
  同一の LevelData 変化を生むこと。
- 既存 autotest 43・全シーンの回帰。

## 実装ステップ

1. walk.rs(判定+補間)+ユニットテスト
2. 3Dモード切替(カメラ/メッシュ spawn・despawn、Camera2d の無効化)
3. 設置/消去(EditCmd 接続+TileDirty)
4. 配置マーカー(アイテム/モンスター/トリガー)+編集反映
5. 座標表示・フロア連動・Undo/Redo の3D反映確認
6. editor-3d シーン+総回帰

## 受け入れ基準

1. ビルド完走、clippy 警告なし、`cargo test` 全通過(walk 系追加分含む)。
2. `DEEPGRID_DEBUG_SHOT=editor-3d` で「ダンジョン内の一人称視点+設置済み
   パーツのマーカー」が写る(mtime 確認)。既存シーン・autotest 43 全通過。
3. 手動確認: 2D⇔3D切替、WASD/QE/R/F の歩行、左右クリックでの設置/消去、
   Undo/Redo が3D表示に反映、隠しワープが3Dで見える、Save All で保存されて
   プレイモードに反映される。
4. sample プロジェクトで「3Dだけで小部屋を1つ増築する」ことができる。

## 実装上の注意

- 編集は必ず既存 EditCmd 経由(2Dと同一経路)。3D専用のデータ変更コードを
  書かない — Undo/dirty/警告の一元化(plan9 の最重要原則)を守る。
- egui のフォーカスガード(`wants_keyboard_input` / `wants_pointer_input`)を
  忘れない(テキスト入力中に WASD で歩き出す事故の防止)。
- メッシュ・マーカーは `Edit3dScoped` で必ず括り、2D復帰時に leak しない。
- lavapipe 負荷: モンスターのアニメ再生なし・全フロア表示は plan2 以来の
  描画方式のままで問題ないが、重ければ「上のフロアを非表示」オプションを
  検討(まず計測)。
- `ClusterConfig::Single` は3Dエディットカメラにも適用(ライトを置く場合)。
  基本はプレイと同じ AmbientLight+ヘッドランプ構成を流用する。

## 実装状況(2026-07-15 検証済み)

- 計画どおり実装(walk.rs / edit3d.rs 新設、editor::run 拡張、editor-3d
  シーン)。clippy 警告ゼロ、cargo test 59件、autotest 43ステップ、
  editor-3d / editor-map / プレイシーンの撮影・目視まで全通過。
- 計画外の改善: マップタブの Undo に配置リスト(アイテム/モンスター/
  イベント雛形)の before→after も載せた。従来スナップショット頼みだった
  配置編集が、2D/3D 共通の EditOp で undo/redo できる
  (`trigger_place_is_one_undo_op` 等のテストで担保)。
- 統合時の差し戻し: 手動受け入れテストの Save All で sample プロジェクトの
  RON 全ファイルが再シリアライズされていた(コメント消失+テスト編集の
  残骸: floor2 の鍵穴雛形・壁数個・壁内モンスター配置など)。定義4ファイル
  は意味的に無変更と確認の上、assets/projects/sample/ を全て HEAD に復元
  した(受け入れ基準4は「増築できること」の確認であり、結果の commit は
  不要)。
