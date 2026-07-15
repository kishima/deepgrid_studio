# plan10: 演出と仕上げ第1弾 — 音・デモ・グラフィック差し替え・セーブ

## この文書について

第10実装計画書(2026-07-15、plan9.5 完了後の実状に合わせて全面改訂)。
仕様源は [project.md](project.md)「演出・その他」、
[dandan_spec_event.md](dandan_spec_event.md)(デモの起動)、
[dandan_spec_mapeditor.md](dandan_spec_mapeditor.md)「その他の処理」。
開発環境の制約は [plan1.md](plan1.md)。素材導入時は
リポジトリ直下 CLAUDE.md の**素材の記録ルール(必須)**に従うこと。

## ゴール

音(BGM・効果音)、デモ(OP/ED/中間)、グラフィック差し替え、セーブ/ロード
を実装し、「作ったゲームを最初から最後まで遊べて、続きから再開できる」状態
にする。

## 現状(plan9.5 完了時点の実状)

- **音**: Cargo の bevy features に音響系は意図的に不在(plan1の判断)。
  追加するのは `"bevy_audio"` と `"vorbis"`(ogg)。assets/audio/ は未作成。
- **イベント**: `ChangeBgm` / `StartDemo` は event.rs の実行部で
  「(未実装)」とログに出すだけのスタブ(event.rs 550行台)。
- **プロジェクト**: `PROJECT_VERSION = 6`。project.ron + levels/*.ron +
  characters/items/monsters/magics.ron。`LimitsConfig.demo_message_lines`
  (=160)だけ先行して存在する。デモ・BGM のデータ構造は無い。
- **時間**: clock.rs `CYCLE_SECS = 0.1` 固定、`GameClock { cycle, accum }`、
  CycleTick イベント駆動。速度倍率の仕組みは無い。
- **乱数**: rng.rs `GameRng`(xorshift64*、固定シード)。state は private
  (セーブには getter/setter の追加が要る)。
- **セーブ対象になる既存状態**(全部ある — plan10 はこれらの直列化が主):
  - `Player { pos, facing }`(movement.rs)+ `MoveMode`(event.rs)
  - `Party`(members: Character + CharacterState + Inventory。習得魔法
    learned・満腹度 satiety・effects も CharacterState 内)
  - `CurrentLevel` / `LevelStates`(訪問済みレベルの MonsterSnapshot・
    FloorItemSnapshot・doors_open・TriggerStates・block_diffs — plan8 の
    レベル遷移スナップショットがそのまま流用できる。現在レベルの
    スナップショット処理は world.rs の遷移ハンドラ内にあるので
    関数に切り出す)
  - `EventFlags` / `EventQueue`(fire_cycle は絶対サイクルなので
    GameClock.cycle と一緒に保存すれば整合する)/ `WallWrites`
  - `GameClock.cycle` / `GameRng` の内部状態
- **ユーザー設定**: settings.rs が `user_settings.ron`(リポジトリ直下)を
  既に読み書きしている(キーバインド)。音量等はここに相乗りできる。
- **差し替えの下地**: ポートレートは `"projects/sample/portraits/…"` という
  プロジェクト内アセットパスを既に asset_server で読んでいる(assets/ が
  ルート)。地形テクスチャは render::build_palette 内のパス直書き。
  アイテム/モンスターのモデルパスは元々プロジェクトデータ(差し替え可能)。
- **エディター**: Tab enum 7枚(Map〜Settings)+ labels.rs(表示名一元化)。
  コンテンツ系は全プロジェクトスナップショットで Undo。
- **検証**: autotest **43ステップ**。シーンはプレイ17種
  (1/fall/ladder/door/monster/magic/light/potion/plate/warp/stairs/hole/
  combat/items/pickup/data/liquid)+エディター7種+editor-3d。
  egui は EguiRenderToImage、bevy_ui は Bevy Screenshot に写る。
- **空きキー**(プレイ側): 使用中は WASD/QE/R/F/矢印/Space/Tab/B/C/G/I/M/
  T/V/O/Esc。J/K/L/N/P/U/X/Z あたりは空いている。

## スコープ

### やること

1. **音響基盤**: bevy features に `"bevy_audio"` + `"vorbis"` を追加。
   BGM/SE の音量(0.0〜1.0)とミュート・足音ON/OFF を user_settings.ron に
   保存。**音声デバイスが無い環境(docker/CI)でも警告のみで動作継続**する
   ことを必須要件とする(Bevy は既定でそう振る舞うはず — 確認する)。
2. **BGM**: `LevelData.bgm: String`(#[serde(default)]、空=無音)を追加し
   レベルごとに1曲。`ChangeBgm` を実体化(次のレベル移動 or 次の ChangeBgm
   まで有効な上書き)。切替は約1秒のクロスフェード(AudioPlayer 2本の
   音量ランプ)。現在曲は `BgmState` リソースに持つ(autotest はこれを
   検証する — 実音は聞けない)。エディターのマップタブに BGM 選択を追加。
   デフォルト曲は CC0 優先で **5〜8曲**導入(オリジナルの17曲は枠として
   扱い、全部は埋めない)。ogg 合計 20MB 以下。
3. **効果音**: 約10種を CC0 で導入: 足音(設定でOFF可)、ドア開閉、攻撃
   ヒット、詠唱、光弾着弾、落下着地、拾う、レベルアップ、スイッチ/
   しかけ床、ワープ。フックは既存システム(movement / combat / magic /
   event / floor_items)の該当箇所に直接置く(汎用SEイベントバスは作らない
   — 過剰設計)。
4. **デモシステム**: 新ファイル `demos.ron`(`Vec<DemoDef>`)。
   `DemoDef { id, name, lines: Vec<String>, bgm: String, bg_color }`。
   行数上限は既存 `limits.demo_message_lines`、本数上限に
   `limits.max_demos`(serde default 6 = OP/ED/中間4 の目安)を追加。
   再生は bevy_ui の全画面オーバーレイ(黒背景+1行ずつ送るテキスト)。
   再生中は `DemoState` リソースで移動・モンスターAI・ハザード・サイクル
   進行を停止。Escape/クリックでスキップ。`StartDemo` を実体化。
   エンディング扱いは暫定: 最終行まで表示→「END」表示→入力待ち→
   メイン画面復帰(タイトル画面は plan11 で差し替え)。
   **デモエディター**: `Tab::Demos` を追加(左一覧+右詳細の共通
   レイアウト。複数行 TextEdit、BGM選択、行数の上限警告)。
5. **グラフィック差し替え機構**: プロジェクト直下 `override/` に、組み込み
   アセットと同じ相対パスで置いたファイルが優先される
   (例: `assets/projects/sample/override/textures/wall_bricks077_color.png`)。
   パス解決は project.rs の1関数に集約(`fn resolve_asset(project, rel)`:
   override にファイルが実在すればそのアセットパス、なければ組み込み)。
   適用対象は **build_palette が読む地形テクスチャ一式**(壁/書ける壁/床/
   天井/ドア/はしご/階段/液体)。アイテム・モンスターのモデルと
   ポートレートは既にプロジェクトデータでパス指定できるので対象外
   (ドキュメントにその旨を書く)。エディターの設定タブに
   「差し替え検出一覧」(override/ を走査して表示)を追加。
6. **セーブ/ロード**: `<プロジェクト>/saves/slot{1..3}.ron`。
   `SaveData`(`save_version: u32 = 1`、不一致は拒否+メッセージ):
   現状セクションに列挙した全状態。手順は「現在レベルを LevelStates へ
   スナップショット(切り出した関数を再利用)→ LevelStates ごと直列化」。
   必要な構造体に Serialize/Deserialize と serde(default) を追加
   (LevelState / MonsterSnapshot / FloorItemSnapshot / TriggerStates /
   CharacterState / Inventory / ItemInstance など)。GameRng に state の
   取得/復元を追加。UI はデータ画面に「セーブ/ロード」ボタン(スロット
   3つ+空き表示。マウス操作、新規キーは割り当てない)。CLI
   `--load <slot>` も受ける(autotest・再現用)。
7. **ゲーム速度**: user_settings.ron に `speed`(0.5/1.0/2.0)を追加し、
   tick_clock で `delta × speed` として一元適用(個々のタイマーは触らない。
   移動アニメ秒数はゲーム時間でなく演出なのでそのまま)。
   設定変更UIは plan11(タイトル/オプション)まで RON 直編集でよい。

### やらないこと

- タイトル画面・プロジェクト選択・配布形式・Windows 実行(plan11)
- 魔法シンボルの画像差し替え(シンボルは文字グリフであり画像ではない)
- アイテム表示形式(簡易/詳細)設定(データ画面は現代化済みで該当なし)
- BGM 17曲フルセット(枠のみ。曲は後からでも置ける)
- 音量・速度の変更UI(plan11 のオプション画面で)
- セーブデータの後方互換マイグレーション(バージョン拒否のみ。plan11以降)

## 設計メモ

- **PROJECT_VERSION 6→7**: 追加は全て #[serde(default)] で旧プロジェクトが
  そのまま読めること(既存の後方互換テストのパターンを踏襲し、v6 の
  sample が demos 無しで読めるユニットテストを足す)。
- **サンプルへの実データ**: sample プロジェクトに BGM 設定(2レベル)、
  OP デモ1本+中間デモ1本、StartDemo/ChangeBgm を使うイベントを追加する
  (autotest のフィクスチャを兼ねる)。既存イベント座標は動かさない。
- **決定論**: セーブに GameRng の state を含めるので、
  「セーブ→N歩→状態A / ロード→同じN歩→状態A」が成立する。autotest で
  この完全一致(位置・HP・フラグ・乱数状態)を検証する。
- **BGM/SEの実音**: docker では聞けない。検証は BgmState / 再生エンティティ
  の存在を autotest で確認し、実際に鳴ることはユーザーの手動確認1回に
  任せる(WSLg は PulseAudio が通るはず。鳴らなければ報告してもらう)。
- **素材の記録**: 音源は導入と同じコミットで CREDITS.md +
  assets/audio/README.md に記録。CC0 優先(Kenney 等)。CC-BY を使う場合は
  帰属表示先がまだ無いので README/CREDITS 記載で暫定(plan11 のタイトルで
  クレジット画面に載せる)。**アーカイブ原本はコミットしない。**

## 実装ステップ

1. 音響基盤(features/設定/無デバイス耐性)+ SE 10種導入・記録
2. BGM(LevelData.bgm、BgmState、クロスフェード、ChangeBgm 実体化、
   エディターUI、曲導入・記録)
3. セーブ/ロード(スナップショット関数の切り出し→ SaveData →
   データ画面UI → --load。ラウンドトリップのユニットテスト必須)
4. デモ(demos.ron、DemoState、再生オーバーレイ、StartDemo 実体化、
   Tab::Demos エディター)
5. グラフィック差し替え(resolve_asset、build_palette 経路の置換、
   エディター一覧)
6. ゲーム速度(user_settings.speed)
7. 検証: 新シーン `demo`(デモ再生中のオーバーレイ)/`override`
   (壁テクスチャ差し替え状態)/`editor-demos`(egui)。autotest に
   bgm-change / demo-start-skip / save-load(決定論一致)/ speed の
   ステップを追加(43→50前後を想定)

## 受け入れ基準

1. ビルド完走、clippy 警告なし、`cargo test` 全通過(セーブの
   ラウンドトリップ、v6 プロジェクトの後方互換を含む)。
2. autotest 全ステップ PASS(音声デバイス無しの docker で完走すること
   自体が無デバイス耐性の確認になる)。
3. `DEEPGRID_DEBUG_SHOT=demo|override|editor-demos` 撮影(mtime 確認)+
   既存シーン全通過。
4. 手動確認(ユーザー): BGMがレベル移動で切り替わる(クロスフェード)、
   効果音が鳴る、OPデモ→ゲーム→中間デモの一巡、セーブ→終了→
   `--load 1` で続きから、override/ に壁テクスチャを置くと見た目が変わる。
5. 導入した音源が全件 CREDITS.md + assets/audio/README.md に記録され、
   原本アーカイブがコミットされていないこと。

## 実装上の注意

- ユーザー設定(音量・速度・足音)とプロジェクトデータを混ぜない。
  前者は user_settings.ron、後者は project.ron/demos.ron。
- セーブへ書く構造体には必ず #[serde(default)] を付け、将来フィールドに
  強くする。SaveData 直下に save_version。
- デモ再生中の停止は「システムを止める」のではなく DemoState を各システムが
  見る形にする(autotest / debug_shot のドライバは動き続ける必要がある)。
- 差し替えのパス解決は project.rs の resolve_asset に一本化し、
  ランタイム・エディター両方が同じ関数を通る。Palette 構築時に1回解決
  すれば十分(ホットリロードはしない)。
- 効果音の同時多発(モンスター多数のヒット等)は同一SEの多重再生を
  1フレーム1回に間引く程度でよい(音響エンジンは作らない)。
- 新規ファイルを含むコミットでは git status を確認し、並行作業の
  ステージ済み変更を巻き込まないこと。
