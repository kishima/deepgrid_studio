# plan12: Bevy States への全面移行(機能追加なし)

## この文書について

第12実装計画書(2026-07-16、plan11 完了後の実状に合わせて作成)。
**純リファクタ planであり、ユーザーから見える挙動の変化はゼロが要件。**
仕様源は無し(内部構造の変更のみ)。位置づけは roadmap.md のとおり
「plan11 完了後・第2期の画面追加前・Bevy バージョン更新 plan より前」。

## ゴール

画面遷移(タイトル/デモ/プレイ)を Bevy の `States` に載せ替え、
第2期で画面(オプション、wasm メニュー等)を追加するときに
`OnEnter/OnExit` と `run_if(in_state(...))` で書ける土台を作る。
`scripts/verify-all.sh` 全35項目が**無修正またはそれに準ずる最小修正で**
通ることをもって「挙動不変」とする。

## 現状(plan11 完了時点の実状)

- 画面の種類と優先則は `src/screen.rs::active_screen` に集約済み:
  **Title > Demo > Data > Play**(enum `ActiveScreen`+`CurrentScreen`
  SystemParam)。plan11 でこの形に整えたのは本 plan のため。
- ゲートの実体はリソース: `TitleState.active` / `DemoState`(playing)/
  `DataScreen.open`。
- `CurrentScreen` / `active_screen` の消費者は**3システムだけ**:
  - `clock.rs::tick_clock`(Title/Demo でクロック凍結。モンスターAI・
    ハザード等は CycleTick 駆動なので、クロックが止まれば連鎖的に止まる —
    個別ゲート不要の設計が既にできている)
  - `player/movement.rs`(Title/Demo で入力・アニメ凍結)
  - `settings.rs::keyconfig_input`
- 遷移を書くのは `title.rs`(メニュー操作・ResetRunReq)と
  `demo.rs::drive_demo`(クローズ分岐: 通常→プレイ復帰、`"ed"`→リセット+
  タイトル)。タイトルUIは「状態が変わるたび despawn_recursive+再構築」。
- 起動時の初期画面: 通常はタイトル、`DEEPGRID_AUTOTEST` /
  `DEEPGRID_DEBUG_SHOT` / `DEEPGRID_PERF` / `--load` は直行(タイトルなし)。
- autotest(49ステップ)は step48/49 でタイトル遷移を**実キー注入**で検証
  (`title.active` / `demo.playing()` を跨フレームでポーリング)。
- `DataScreen.open` は上記と性質が違う: movement.rs のコマンド処理内で
  **同一フレーム中に書いて読む**(Tab トグル→直後の分岐)箇所が多数あり、
  hud / magic / autotest も直接 open を操作する。データ画面中も世界は
  進み続ける(plan5 の仕様)。
- エディターは別 App(egui)で画面遷移を持たない — 本 plan の対象外。

## スコープ

### やること

1. **`GameScreen` States の導入**(screen.rs を置き換え):
   ```rust
   #[derive(States, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
   pub enum GameScreen { #[default] Title, Demo, Playing }
   ```
   - 起動時は `app.insert_state(...)` で初期状態を選ぶ(無人モード/
     `--load` は `Playing`、通常は `Title`)。
   - `TitleState.active` / `DemoState` の「再生中かどうか」ビットを廃止し、
     残りのフィールド(メニュー選択、デモ進行、エラーバナー等)は
     データ持ちリソースとして存続。
2. **DataScreen は States にしない(意図的な設計判断)**:
   データ画面は「世界が進み続けるプレイ中のオーバーレイ」であり
   (plan5 仕様)、同一フレームで open を書いて読む入力ルーティングが
   挙動を担っている。`NextState` は次の StateTransition まで反映されない
   ため、State 化は同一フレーム意味論を壊す=挙動変更になる。
   モデルとしてもオーバーレイが正しい。**screen.rs の
   `ActiveScreen::Data` は「GameScreen::Playing かつ data.open」の
   導出値として残す**(優先則の一元化は維持)。
   ※roadmap の plan12 行の「DataScreen 等を統一」はこの設計に合わせて
   修正すること。
3. **ゲートの置き換え**:
   - `tick_clock` / movement の凍結 / `keyconfig_input` →
     `run_if(in_state(GameScreen::Playing))`(または関数内 State 参照。
     **他システムとの順序制約が既にある場合は関数内参照を優先**し、
     `.after()` チェーンを崩さない)
   - タイトルUI・デモオーバーレイの構築/破棄 → `OnEnter` / `OnExit`
     スケジュールへ移設(「毎フレーム despawn+再構築」の現行タイトルは
     `OnEnter(Title)` 構築+状態変化時の再構築+`OnExit(Title)` 破棄に整理)
4. **遷移書き込みの置き換え**: `title.rs` / `demo.rs` / セーブロード経路 /
   `apply_reset` の「リソースのフラグ反転」を `ResMut<NextState<GameScreen>>`
   に置換。**遷移が遅延反映になる全箇所を監査**する(下記リスク)。
5. **autotest / debug_shot の追随**: 検証ロジックは変えない。
   `title.active` / `demo.playing()` を見ているアサートを
   `State<GameScreen>` 参照に書き換えるだけ(跨フレームポーリングなので
   遅延反映の影響は受けないはずだが、step48/49 は重点確認)。
6. **ドキュメント**: screen.rs の冒頭コメントと README 開発者向け節を
   States 前提に更新。plan11.md の「plan12 で差し替える」注記はそのまま
   (歴史記録)。

### やらないこと

- 機能追加・UI変更・挙動変更の一切(新しい画面もここでは足さない)
- DataScreen / MoveMode / KeyConfig の State 化(上記のとおり
  オーバーレイ/モード扱いが正しい)
- エディター App への States 導入(画面遷移が無い)
- SubStates / ComputedStates の導入(現状3状態に対して過剰。第2期で
  画面が増えて必要になったときに)
- Bevy バージョン更新(独立 plan。本 plan の後)

## 遅延遷移の監査リスト(最重要リスク)

`NextState` は書いた次の StateTransition スケジュールまで `State` に
反映されない。現行コードは「フラグを書いた同一フレーム内に別システムが
それを読む」可能性がある。以下を1箇所ずつ確認し、同一フレーム依存が
あれば**遷移を書いたシステム自身がその場で必要な後始末までやる**形に
寄せる(次フレームの世界が正しければよい):

1. タイトル「はじめから」→(OPデモありなら)デモ開始: Title→Demo の
   連続遷移。1フレームに2回 NextState を書かない(はじめから=
   `NextState(Demo)` 直行 or `NextState(Playing)`)
2. `drive_demo` のクローズ: 通常→Playing / `"ed"`→ResetRunReq+Title。
   apply_reset の実行タイミングと OnExit(Demo) の破棄順
3. つづきから(セーブロード)→ Playing: ロード適用がタイトルを閉じる
   フレームで完了しているか(autotest step49 の検証対象)
4. 壊れたプロジェクトのフォールバック: 起動直後にタイトル+エラーバナー
   (初期状態 Title なので遷移なし — バナーはリソース側)
5. autotest のキー注入: `run_title` より前に注入する現行の順序付けが
   run_if 化後も成立するか

## 実装ステップ

1. `GameScreen` States 導入+初期状態の選択(この時点では従来リソースと
   二重管理で全テスト通過を確認 — 一気に切り替えない)
2. 消費側3システム+タイトル/デモUIの構築破棄を States 参照へ切替
3. 遷移書き込みを NextState へ切替+監査リスト消化、旧フラグ削除
4. autotest / ユニットテストの参照書き換え
5. screen.rs 整理(ActiveScreen は導出ヘルパーとして残す)+ドキュメント
6. `./scripts/verify-all.sh` 全通過+title/demo/saveload 系シーンの目視比較
   (移行前後のスクリーンショットで差分がないこと)

## 受け入れ基準

1. ビルド完走、clippy 警告なし、`cargo test` 全通過。
2. **`scripts/verify-all.sh` 全35項目 PASS**(autotest 49ステップ・
   シーン29種を含む)。autotest のステップ数・シーン数は増減なし
   (機能追加なしの証跡)。
3. `title` / `demo` / `1` シーンの移行前後スクリーンショットに差分がない
   (目視)。
4. コードから `TitleState.active` と DemoState の再生中フラグが消え、
   画面判定が `State<GameScreen>`(+DataScreen オーバーレイ導出)に
   一本化されている。
5. 遅延遷移の監査リスト5項目それぞれの確認結果が本文書の「実装状況」節に
   記録されている。

## 実装上の注意

- **挙動維持が唯一の成果物**。迷ったら「現行の観測可能な挙動」に合わせ、
  改善したい点があっても本 plan では見送って記録だけ残す。
- リファクタ途中の中間コミットでも verify-all が通る状態を保つ
  (二重管理期間を恐れない — 一括切替の巨大差分より安全)。
- `in_state` の run_if を足すとき、`.after()/.before()` の既存チェーンを
  変えない。順序が絡むシステムは関数内で `Res<State<GameScreen>>` を
  読む方式に逃げてよい(plan11 までの「1関数集約」の精神は State 参照に
  なった時点で満たされている)。
- autotest の期待値をリファクタ都合で書き換えない(書き換えが必要に
  なった時点でそれは挙動変更 — 原因を直す)。
- 新規ファイルを含むコミットでは git status を確認し、並行作業の
  ステージ済み変更を巻き込まないこと。
