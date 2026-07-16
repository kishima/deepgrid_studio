# plan13: 統合アプリ — エディター⇔プレイのシームレス切替

## この文書について

第13実装計画書(2026-07-17、plan12 完了後の実状に合わせて作成。第2期の初手)。
仕様源: ユーザー要望「通常起動でエディットとプレイを行き来したい。
配布版はプレイのみ。別アプリの起動し直しは大変」+ plan9.5 で第2期送りに
した「プレイモードとのシームレス切替」。開発環境の制約は [plan1.md](plan1.md)。

## ゴール

1プロセス・1ウィンドウの中で タイトル ⇔ エディター ⇔ プレイ を行き来できる。
エディターの**未保存の編集内容のままテストプレイ**でき、戻ると編集状態
(Undo履歴・タブ・選択)がそのまま残っている。配布版(play_only)では
エディターへの入口が見えない。egui(エディター)と bevy_ui(ゲーム)の
住み分けは継続する(2026-07-17 ユーザー決定)。

## 現状(plan12 完了時点の実状)

- **App が2つある**: main.rs が `--edit` / `DEEPGRID_DEBUG_SHOT=editor-*` で
  `editor::run(project)`(DefaultPlugins+EguiPlugin+Camera2d+3Dエディット)
  か `run_play(...)`(DefaultPlugins+bevy_ui、egui なし)のどちらかを起動。
  行き来はプロセスを立ち上げ直すしかない。
- `GameScreen` States(Title/Demo/Playing)が plan12 で導入済み。
  初期状態は `insert_state`(無人検証/`--load` は Playing、通常 Title)。
  画面の構築/破棄は OnEnter/OnExit、優先則は screen.rs。
- **Project→ランタイムリソースの導出が run_play にインライン**:
  catalogs(item/monster/magic)、GameLevels、InitialItems/Monsters、
  EventFlags 初期値、InitialRun(はじめから用のプリスティン状態)、
  LimitsConfig、RulesConfig、DemoCatalog、TitleState のメタ表示。
  ここが関数化されていないことが「エディターの編集結果で世界を作り直す」
  ことの唯一の本質的障害(後述の中核タスク)。
- **世界の作り直し機構は実証済み**: `apply_reset`(ResetRunReq)と
  `handle_load` が「グローバルを書き戻して `LevelTransition` +
  `SkipNextSnapshot` で再構築」という同型のフローを持つ。テストプレイは
  この第3の亜種になる。
- **リソースの共有と衝突**: `Dungeon` / `Palette` / `TileDirty` /
  `LevelScoped` はプレイと editor::edit3d の両方が使う。今は別 App だから
  衝突しない。統合後は「状態が変わるたび完全再構築」で所有権を渡す。
- エディターの撮影(editor-* 9シーン)は shot.rs の EguiRenderToImage。
  プレイ20シーンは Bevy Screenshot(egui は写らないが bevy_ui は写る)。
- `PlayOnly` リソース+`deepgrid.ron` は plan11 から存在(--edit 拒否済み)。
- タイトルメニューは6項目(はじめから/つづきから/設定/クレジット/
  ゲームを選ぶ/終了)。「エディター」項目は無い。
- 空きキー: J/K/L/N/P/U/X/Z、ファンクションキー全部。
- 検証: `./scripts/verify-all.sh` 全35項目(autotest 49、シーン29)。

## スコープ

### やること

1. **Project→ランタイムリソース導出の関数化**(中核・最初にやる):
   run_play にインラインの導出一式を
   `fn build_runtime(project: &Project) -> RuntimeBundle` のような形に
   抽出し、「App への一括 insert / 既存 App への再 insert」の両方に使える
   ようにする。起動・テストプレイ開始・エディター→タイトル復帰が同じ
   関数を通る(ユニットテスト対象)。
2. **App 統合**: main.rs で単一 App を構築(EguiPlugin 常時登録、
   エディター系システムは `run_if(in_state(GameScreen::Editor))`)。
   `editor::run` は廃止し、`--edit` と `DEEPGRID_DEBUG_SHOT=editor-*` は
   **初期状態 Editor** で同じ App を起動する形に変える。
   EguiRenderToImage 撮影(shot.rs)は統合 App 上で従来どおり動くこと。
3. **`GameScreen::Editor` 状態の追加**:
   - タイトルメニューに「エディター」項目(**play_only では非表示**。
     `resolve_mode` 相当の単体テストも追加)
   - OnEnter(Editor): プレイ側エンティティ(LevelScoped・HUD・カメラ・
     ポートレートリグ)を破棄、BGM 停止、Camera2d+EditorState を構築。
     **EditorState は初回だけ生成し、以後はリソースとして保持**
     (再入場で Undo履歴・タブ・選択が残る = 受け入れ基準)
   - OnExit(Editor): エディター側エンティティ(Camera2d・Edit3dScoped)を
     破棄。egui はエディター状態でのみ描く
4. **テストプレイ**: エディター上部バーに「テストプレイ」ボタン+ **F5**。
   - `EditorState.proj`(未保存のまま)から 1. の関数でランタイムを
     再導出 → リセットと同じ経路で世界構築 → `NextState(Playing)`。
     `TestPlay(true)` マーカーリソースを立てる
   - テストプレイ中: **F5 でエディターへ帰還**(メッセージログに起動時
     案内を出す)。**セーブ/ロードは無効**(未保存プロジェクトとディスクの
     セーブが食い違うため。データ画面のスロットはグレーアウト+理由表示)。
     ED デモの帰還先もタイトルでなく**エディター**
   - 帰還時: プレイ世界を破棄して Editor へ。EditorState は触らない
     (テストプレイの結果は一切書き戻さない)
5. **エディター→タイトル**: エディターに「タイトルへ」ボタン。このとき
   ランタイムを **EditorState.proj から再導出**する — アプリ実行中の
   データの正は常にメモリ上の編集内容とする(Save All 済みかどうかで
   タイトルの挙動が変わる方が混乱する)。未保存があれば
   ボタン脇に「未保存」表示(保存の強制はしない)。
6. **検証の拡張**:
   - autotest: (50) タイトル→エディター遷移、(51) EditCmd で壁を1つ
     置いて F5 → プレイ世界にその壁が存在+開始位置に立っている、
     (52) F5 帰還で EditorState の Undo 履歴と dirty が保持されている
   - 新シーン `editor-testplay`(エディターで置いた目印ブロックが
     テストプレイの一人称視点に写る — Bevy Screenshot)
   - play_only のメニュー非表示はユニットテスト
7. **README**: 作る人向け節に「エディター⇔テストプレイの行き来」を追記。

### やらないこと

- egui / bevy_ui の統一(住み分け継続 — 2026-07-17 決定)
- 3Dエディットの現在座標からのテストプレイ開始(常にスタート地点から。
  座標引き継ぎは次の改善候補として記録)
- 通常プレイ(タイトル経由)への F5 中断メニュー(テストプレイ専用。
  プレイ中のポーズメニューは第2期の別項目)
- テストプレイ結果の編集への書き戻し(プレイは常に使い捨て)
- エディター単独の配布形態(配布は従来どおり play_only)

## 設計メモ

- **状態遷移図(plan13 後)**:
  ```
  Title ──はじめから──▶ Demo/Playing(従来どおり)
    │▲                     │
    │└──タイトルへ──── Editor ◀─F5─▶ Playing(TestPlay)
    └──エディター──────▶┘        (EDデモ帰還も Editor)
  ```
- **所有権の渡し方は「完全再構築」一択**: プレイ⇔エディターの切替で
  メッシュ・配置物は必ず despawn→rebuild(plan8 のレベル遷移、plan11 の
  リセットと同じ思想)。「隠して使い回す」最適化はしない(状態の
  持ち越しバグの温床。lavapipe でも再構築は一瞬なのは実証済み)。
- **TestPlay 中の Playing は通常の Playing と同一システム構成**
  (差分は TestPlay リソースを見る 3点だけ: F5 帰還、セーブ無効、
  ED 帰還先)。テストプレイ専用の世界構築コードを作らない。
- EditorState 生成時の Project は起動時ロードのものを使い、以後
  アプリ内で編集された `EditorState.proj` が唯一の正。ディスクとの同期は
  従来どおり Save All のみ。
- `--edit` の意味は「初期状態 Editor」に変わるだけで CLI 互換は維持。
  エディターのウィンドウタイトル変更(「— Editor」)は廃止してよい
  (1ウィンドウに統合されるため)。

## 実装ステップ

1. build_runtime 抽出(挙動不変。verify-all 全通過を確認してから次へ)
2. App 統合+editor-* シーンの新経路(ここでも verify-all 全通過 —
   エディターが統合 App で従来どおり動く中間マイルストーン)
3. GameScreen::Editor+タイトル項目+OnEnter/OnExit の所有権移譲
4. テストプレイ(F5 往復、セーブ無効、ED 帰還先)
5. エディター→タイトル(再導出)+play_only 非表示
6. autotest 3ステップ+editor-testplay シーン+README
7. verify-all 総回帰+目視(editor / editor-3d / title / testplay)

## 受け入れ基準

1. ビルド完走、clippy 警告なし、`cargo test` 全通過(build_runtime、
   play_only メニュー非表示、TestPlay 中セーブ拒否のテストを含む)。
2. `scripts/verify-all.sh` 全項目 PASS(autotest 49→52、シーン 29→30)。
3. 手動確認(ユーザー): 通常起動 → タイトル「エディター」→ 壁を数個
   置いて F5 → その壁のあるダンジョンを歩ける → F5 → Undo で壁が消せる
   (履歴が生きている)→「タイトルへ」→「はじめから」で編集後の
   マップが遊べる。ウィンドウは一度も閉じない。
4. `export-game.sh` の成果物(play_only)でタイトルに「エディター」が
   出ず、`--edit` も従来どおり拒否される。
5. テストプレイ中にデータ画面のセーブスロットが無効化され、理由が
   表示される。

## 実装上の注意

- ステップ1・2 はそれぞれ単独で verify-all が通る中間コミットにする
  (App 統合は差分が大きい — 挙動不変の区間を細かく刻む)。
- `Dungeon` / `Palette` / `TileDirty` の書き手が状態ごとに一人になる
  ことを OnEnter/OnExit で保証する(plan9.5 の Edit3dScoped despawn 精神)。
  特に edit3d はプレイと同じ `Dungeon` リソースを上書きするので、
  Editor→Playing 遷移では必ず build_runtime 由来の値で作り直す。
- egui のフレーム処理(bevy_egui)はエディター状態以外でコストゼロに
  近いが、プレイシーンのスクリーンショット29種に視覚差分が出ないことを
  ステップ2の時点で確認する。
- autotest のタイトル/エディター遷移ステップは plan12 と同じ
  「実キー注入+跨フレームポーリング」方式で書く(NextState の遅延反映に
  依存しない)。
- 新規ファイルを含むコミットでは git status を確認し、並行作業の
  ステージ済み変更を巻き込まないこと。
