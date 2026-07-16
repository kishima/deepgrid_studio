# plan11: 配布と品質 — タイトル・配布形式・Windows GPU実行・仕上げ

## この文書について

第11実装計画書(第1期の最終。2026-07-16、plan10 完了後の実状に合わせて
全面改訂)。仕様源は [project.md](project.md)「開発環境」(Windows ネイティブ
GPU 実行の節に導入手順・スクリプト仕様が既に書いてある — 本plan で実体化)
ほか全体。開発環境の制約は [plan1.md](plan1.md)。

## ゴール

「作る人」と「遊ぶ人」を分離する。タイトル画面からゲームを始められ、
作ったゲームをランタイム同梱で配布でき、Windows では GPU でまともな速度で
動く状態にして第1期を締める。

## 現状(plan10 完了時点の実状)

- `PROJECT_VERSION = 7`。CLI は `--edit` / `--project <dir>` /
  `--load <slot>`(save.rs: SLOTS=3, SAVE_VERSION=1,
  `PendingCliLoad`)。不明引数は警告して無視。
  **プロジェクトのロード失敗は現状 panic**(main.rs)。
- タイトル画面は無く、起動すると即ゲーム(または `--edit` でエディター)。
  Bevy の States は使っておらず、全画面オーバーレイは
  **`DemoState` リソースでのゲート**が確立パターン(demo.rs。bevy_ui は
  Bevy Screenshot に写る)。
- デモ: `DemoCatalog` + `start_demo()`。sample には id `"op"` / `"mid"` の
  デモがある。デモ終了は常にゲームへ復帰(ED をタイトルへ返す先が無い)。
- 設定: `UserSettings`(bgm/se 音量・ミュート・足音・ゲーム速度)が
  `user_settings.ron` に保存済み。**変更UIは無い**(plan10 で本plan送り)。
  キーバインド変更は O キーの `KeyConfig` フローが既にある。
- クレジット素材情報は CREDITS.md に全件記録済み(現時点で CC-BY は無し、
  CC0 / PD / M+ のみ)。
- 差し替え: `project::AssetResolver` / `resolve_asset`(`<project>/override/`)。
- エディターの Save All は `*.ron.bak` バックアップを書く。プレイは
  `<project>/saves/` を書く(gitignore 済み)。**配布物からはどちらも除外**。
- Windows ネイティブ実行: project.md に手順仕様あり。
  `.cargo/config.toml` も `scripts/deepgrid-run-win.sh` も**未作成**。
  mingw 導入(`sudo apt install gcc-mingw-w64-x86-64` と
  `rustup target add x86_64-pc-windows-gnu`)は sudo が要るため人間に依頼。
- 検証: autotest **47ステップ**、シーン**28種**(プレイ19+エディター9。
  一覧の正は README「検証用スクリーンショット」節)。一括実行スクリプトは
  無い(毎回手で回している)。
- assets/ は約43MB(音源7曲込み)。フォントは PixelMplus +
  bevy `default_font`。

## スコープ

### やること

1. **タイトル画面**: 起動時に全画面 bevy_ui オーバーレイ(`TitleState`
   リソースでゲート — DemoState と同じパターン。States 導入はしない)。
   メニュー(キーボード上下+Enter、マウス両対応):
   - **はじめから**: id `"op"` のデモがあれば再生してからゲーム開始
     (無ければ即開始)
   - **つづきから**: スロット1〜3の一覧(空き/保存日時を表示)→ 既存の
     ロード経路で復帰
   - **設定**: bgm/se 音量・ミュート・足音・ゲーム速度(0.5/1/2)の変更UI
     (UserSettings に即保存)+「キー設定」(既存 KeyConfig フローへ)
   - **クレジット**: CREDITS.md の内容をスクロール表示(実行時に
     ファイルから読む。無ければ項目ごと非表示)
   - **ゲームを選ぶ**: 現プロジェクトの親ディレクトリを走査して
     project.ron を持つディレクトリを一覧 → 選択で **自プロセスを
     `--project <dir>` 付きで再実行して終了**(リソース初期化は起動時
     一括の現行構造を崩さない。ウィンドウは開き直しになるが許容)
   - **終了**
   - `DEEPGRID_AUTOTEST` / `DEEPGRID_DEBUG_SHOT` / `--load` 指定時は
     タイトルを出さず直行(既存の無人検証を一切壊さない)。
2. **EDデモ→タイトル**: id `"ed"` のデモは再生終了後、ゲームでなく
   タイトルへ戻る(パーティ・世界は初期状態で作り直し=はじめからと同じ
   経路)。これが「ゲームクリア」の暫定形。
3. **プロジェクト掲載メタデータ**: project.ron に `author` / `description`
   (#[serde(default)])を追加し、タイトルと「ゲームを選ぶ」一覧に表示。
   `PROJECT_VERSION 7→8`(v7 が読める後方互換テスト付き)。
   エディターの設定タブに編集欄を追加。
4. **配布形式**: `scripts/export-game.sh <project-dir> <out-dir>`:
   - コピー: リリースバイナリ(linux。`deepgrid_studio.exe` がリポジトリ
     直下にあれば同梱)+ `assets/` 一式(**ただし assets/projects/ は
     対象プロジェクトのみ。saves/ と *.ron.bak は除外**)+ CREDITS.md +
     遊ぶ人向け README(スクリプトが生成)
   - `deepgrid.ron`(起動設定)を出力先直下に生成:
     `( play_only: true, project: "assets/projects/<name>" )`。
     ランタイムは起動時にカレントの `deepgrid.ron` があれば読み、
     プロジェクトを固定+**play_only では `--edit` と「ゲームを選ぶ」を
     無効化**(丁寧に断るだけでよい。DRM 的な厳密さは不要)
   - 参照アセットの抽出はしない(assets/ 丸ごと約43MB を許容。走査方式は
     漏れリスクの割に節約が小さい)。zip 化は手動でよい。
5. **Windows ネイティブ GPU 実行**(project.md の手順仕様を実体化):
   - `.cargo/config.toml`: `x86_64-pc-windows-gnu` のリンカ指定をコミット
   - `scripts/deepgrid-run-win.sh`: `CARGO_TARGET_DIR=~/.cache/deepgrid-target-win`
     でクロスビルド → exe をリポジトリ直下へコピー(.gitignore 追加)→
     interop で起動。`DEEPGRID_*` / `WGPU_*` を **WSLENV** で橋渡し
     (env を増やしたら列挙にも足すこと)。起動ログの AdapterInfo を
     必ず表示する
   - mingw / rustup target の導入コマンドは**人間に依頼**(sudo)
   - リスク: 音響(cpal/WASAPI)の mingw クロスリンクは未検証。リンクで
     詰まったら「Windows ビルドのみ音無し feature」で逃げず、まず
     エラーを報告して相談すること
   - 相対パス I/O(プロジェクト・saves/・debug-shot.png・user_settings.ron)
     が UNC な cwd(\\wsl$\…)でも動くことを確認
6. **パフォーマンス**: 計測してから直す。
   - `DEEPGRID_PERF=1`: 起動後 N 秒間の平均/最悪フレーム時間を標準出力に
     出して終了する計測モードを追加。上限いっぱい(40×40×5、
     モンスター・アイテム多数)の計測用プロジェクトを
     `scripts/gen_stress_project.py` で生成(スクリプトはコミット)
   - 目標: lavapipe(docker)で 20fps 以上 / Windows GPU で 60fps 張り付き
   - 未達のときの優先手当(計測で効くものだけ): 床タイルの矩形マージ、
     モンスターのアニメ更新間引き、「上のフロアを非表示」オプション
     (plan9.5 で保留した案)。**着手前後の計測値を本文書に追記する**
7. **品質仕上げ**:
   - パニック撲滅: プロジェクトのロード失敗・壊れた RON は panic せず、
     タイトルにエラーバナーを出して「ゲームを選ぶ」へ誘導(タイトルすら
     出せない場合のみ stderr + 終了コード1)。`--load` の壊れたセーブも
     メッセージで拒否(既存挙動の確認)
   - `--help` 追加(全オプションと env 変数の一覧)
   - README を「遊ぶ人向け / 作る人向け / 開発者向け」に再構成
8. **`scripts/verify-all.sh`**: clippy → cargo test → docker build →
   autotest → 全シーン(README の一覧を機械可読に持つ)を順に実行し、
   debug-shot.png の mtime を毎回検査、PASS/FAIL 一覧と終了コードを返す。
   以後の plan 完了検証はこれ1本を基準にする。

### やらないこと(第2期候補 — roadmap.md に「第2期」節を作って転記)

- Web(wasm)ビルド、Steam 等への対応
- ゲームパッド対応
- 多言語化(エディターは labels.rs 差し替えの下地のみ既にある)
- リプレイ・実績・ネットワーク
- セーブデータの後方互換マイグレーション
- Bevy バージョン更新(独立 plan)

## 実装ステップ

1. タイトル画面(TitleState ゲート+メニュー+無人検証の直行維持)
   +シーン `title` 追加
2. EDデモ→タイトル、はじめから→OPデモ
3. 設定UI・クレジット表示・プロジェクト選択(再実行方式)
4. project.ron v8(author/description)+エディター編集欄+後方互換テスト
5. play_only(deepgrid.ron)+ export-game.sh(エクスポート先だけで
   起動・プレイできることの検証込み)
6. Windows ネイティブ(config.toml / run-win.sh。mingw 導入は人間へ依頼、
   GPU での体感・AdapterInfo 確認も人間の目視項目)
7. DEEPGRID_PERF + ストレスプロジェクト生成 + 計測(必要なら手当)
8. パニック撲滅・--help・README 再構成
9. verify-all.sh + 総回帰(autotest にタイトル関連ステップを追加:
   ed デモでタイトルへ戻る、play_only で --edit が拒否される、
   つづきから経路)

## 受け入れ基準

1. ビルド完走、clippy 警告なし、`cargo test` 全通過(v7 プロジェクトの
   後方互換、export の除外規則を含む)。
2. `scripts/verify-all.sh` が全項目 PASS(autotest 47+新規ステップ、
   シーン28+`title`)。既存の無人検証がタイトル追加後も直行で動くこと。
3. 手動確認(ユーザー): タイトルから sample で
   はじめから(OP)→プレイ→セーブ→つづきから→(EDデモ発火)→タイトル
   の一巡がマウスだけ/キーボードだけの両方でできる。設定変更が即反映され
   user_settings.ron に残る。
4. `export-game.sh` の成果物を別ディレクトリへ移して単体で起動・プレイ
   でき、`--edit` が丁寧に拒否される。成果物に saves/ と *.ron.bak が
   含まれない。
5. Windows ネイティブ実行で AdapterInfo が NVIDIA + Vulkan になり、
   docker 経路より明確に軽い(人間確認)。音も鳴る。
6. 壊れた project.ron を与えても panic せず、タイトルのエラーバナーに
   落ちる。
7. DEEPGRID_PERF の計測値(lavapipe / GPU)が本文書に追記されている。

## 実装上の注意

- タイトルは DemoState と同じ「リソースでゲートして各システムが見る」
  方式で作る(States への全面移行は第1期ではやらない — 影響範囲が広く
  リグレッションリスクに見合わない)。
- 「ゲームを選ぶ」の再実行方式は `std::process::Command` で自分を spawn
  して即 exit(current_exe + 引数)。Windows exe でも動くことを確認。
- export-game.sh は bash のみで書く(cargo/docker に依存しない。
  バイナリは既存ビルドを使い、無ければエラーで案内)。
- クレジット表示は CREDITS.md を実行時読み込み(export がコピーする)。
  ビルド埋め込みにしない(素材更新のたびに再ビルドさせない)。
- 計測前に最適化しない。手当は1つ入れるごとに DEEPGRID_PERF で効果を
  数値確認し、効かなければ戻す。
- Windows 経路の目視確認(AdapterInfo・音・体感)は人間タスクとして
  完了報告に明記して依頼すること。
- 新規ファイルを含むコミットでは git status を確認し、並行作業の
  ステージ済み変更を巻き込まないこと。
