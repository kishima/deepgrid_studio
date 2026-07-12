# plan3: プロジェクト形式とマップエディター最小版

## この文書について

DeepGrid Studio の第3実装計画書。前提知識は [project.md](project.md)(仕様)、
[roadmap.md](roadmap.md)(全体計画)、[plan1.md](plan1.md)/[plan2.md](plan2.md)
(実装済み)。実装者はこの文書だけで着手できることを意図している。
開発環境の制約(docker 必須、`CARGO_TARGET_DIR` 指定、`cargo run` はホスト不可)は
plan1.md「開発環境の前提」を必ず先に読むこと。
素材を追加する場合は CLAUDE.md「素材の記録ルール」に従うこと。

## ゴール

「作るゲーム一式」を**プロジェクト**(1ディレクトリ)として定義し、
最小のマップエディターで編集→保存→プレイの一巡を回せるようにする。
これ以降の plan は、テストデータを手書き RON ではなくエディターで作れるようになる。

## 現状(plan3 開始時点)

- ランタイム: 複数フロアのダンジョンを一人称グリッド移動で歩ける。
  落下・はしご・ドア2種・ホロスコープ(一方通行)・液体表示・
  テクスチャ・KayKitモデル表示(src/props.rs、ハードコード配置)まで実装済み。
- マップは `assets/maps/test_level.ron`(形式v2、単一レベル)を起動時に読む。
- `DEEPGRID_DEBUG_SHOT=1|fall|ladder|door|props` の検証シーンあり。
- テストは未導入(`cargo test` の対象なし)。

## スコープ

### やること

1. **プロジェクト形式**(ディレクトリ構造と RON スキーマ)の定義と読み書き
2. 既存テストマップのプロジェクト形式への移行(サンプルプロジェクト化)
3. **起動モード**: プレイモード(既定)とエディットモードの分離
4. **bevy_egui 導入**と 2D 俯瞰マップエディター:
   ブロック配置、フロア/レベル切替、スタート位置設定、保存
5. **Undo/Redo**(編集操作のコマンドスタック)
6. エディターコアロジックの**ユニットテスト**(`cargo test` をホストで実行可能に)
7. 検証シーンのエディター対応(`DEEPGRID_DEBUG_SHOT=editor`)

### やらないこと(後続planへ)

- 3Dエディットモード、プレイ/エディットのシームレス切替 → plan9
- モンスター・アイテム・キャラ等の配置/編集(プロジェクト形式に
  置き場所だけ予約しておく) → plan5 以降
- 上限値(LimitsConfig)のエディターUIからの変更 → plan9
  (プロジェクト形式には最初から含める)
- 矩形選択・コピー&ペースト(Undo/Redo 基盤だけ今回入れる) → plan9
- 複数レベルの新規作成UI(レベル数はプロジェクトファイル手編集でよい。
  切替UIは今回作る)

## プロジェクト形式(本plan での定義)

```
assets/projects/sample/          ← サンプルプロジェクト(コミットする)
├── project.ron                  ← メタデータ + LimitsConfig
└── levels/
    ├── level00.ron              ← レベル0(マップ形式v2と同じフロア積層)
    └── level01.ron              ← (任意。サンプルは最低1レベル)
```

`project.ron`:

```ron
(
    name: "Sample Dungeon",
    version: 1,                       // プロジェクト形式のバージョン
    limits: (
        max_levels: 14,
        floors_per_level: 5,
        floor_width: 40,
        floor_height: 40,
        // ... LimitsConfig の全項目(serde でそのまま読み書き)
    ),
    levels: ["levels/level00.ron"],   // レベルファイルの相対パス、順序=レベル番号
)
```

- `levels/levelNN.ron` の中身は現行マップ形式 v2 と同一
  (width/height/start/floors)。start はレベルごとに持つ
  (レベル間移動は plan8 で扱う。プレイモードはレベル0の start から開始)。
- 既存 `assets/maps/test_level.ron` は `assets/projects/sample/levels/level00.ron`
  へ移動し、`assets/maps/` は削除する。src/props.rs のショーケース配置は
  そのまま(サンプルプロジェクトのマップ座標に依存している旨をコメントで明記)。
- 読み書きは `src/project.rs`(新設)に集約: `load_project(dir)` /
  `save_level(dir, index, &LevelData)`。ランタイム(main.rs)もエディターも
  これを通す。**書き出した RON は再読込で同一データになること**
  (ラウンドトリップ。ユニットテスト対象)。
- 保存時は既存ファイルを `*.ron.bak` に退避してから上書きする(1世代のみ)。

## 起動モード

```
./target/release/deepgrid_studio                    # プレイ(assets/projects/sample)
./target/release/deepgrid_studio --project <dir>    # プレイ(指定プロジェクト)
./target/release/deepgrid_studio --edit             # エディター(sample)
./target/release/deepgrid_studio --edit --project <dir>
```

- CLI引数は `std::env::args` の手動パースでよい(clap 等は入れない)。
- docker/deepgrid-run.sh は追加引数をバイナリへそのまま渡すようにする
  (`./docker/deepgrid-run.sh release --edit` ではなく
  `./docker/deepgrid-run.sh --edit` で release 既定のまま渡せる形が望ましい。
  既存の `debug` 引数との両立は「第1引数が debug/release ならモード、
  それ以外はバイナリへの引数」とする)。

## エディターの仕様

bevy_egui 0.33 を導入(Cargo.toml。mycity-simulator で動作実績のある版)。

画面構成(project.md「UIの方針」の「左に一覧・右に詳細」に準拠):

```
+--------------------------------------------------------------+
| 上部バー: プロジェクト名 | レベル選択 | フロア選択 | Save | Undo/Redo |
+----------------+---------------------------------------------+
| 左パネル       |  中央: 2D俯瞰グリッド(現在フロア)          |
|  ブロック      |   - 1セル=固定ピクセル(ズームは任意実装)  |
|  パレット      |   - egui の painter で矩形描画              |
|  (Wall/Empty/  |   - 左ドラッグ: 選択ブロックを塗る          |
|   Water/Fire/  |   - 右クリック: スタート位置+向きを設置     |
|   Poison/      |   - 下のフロアの壁を薄色で下敷き表示        |
|   Ladder/      |     (支持関係が見えるように)               |
|   Door1/Door2/ |                                             |
|   Horoscope×4) |                                             |
+----------------+---------------------------------------------+
| 下部バー: カーソル座標 (x,y,floor) | 選択中ブロック | 未保存マーク |
+--------------------------------------------------------------+
```

- 描画は egui のみで行う(この画面では3Dカメラ・ダンジョンメッシュは
  スポーンしない)。色はブロック種別ごとの単色+記号
  (H/1/2/矢印)テキストでよい。
- スタート位置の設置: 右クリックでその位置に設置、再右クリックで
  向きを N→E→S→W に巡回。スタート位置は塗りつぶしでは消えない
  (スタートのセルを Wall にする編集は拒否してステータスバーに理由表示)。
- 「保存」は現在レベルのみ書き出す。未保存変更があるときはタイトルに
  `*` を表示。**未保存でも終了できてよい**(終了確認ダイアログは plan9)。

## Undo/Redo の設計

`src/editor/` に UI から独立したコアを置く:

```rust
/// 1回の編集操作。ドラッグ1ストローク分のセル変更をまとめて1オペレーション。
pub struct EditOp {
    pub cells: Vec<(GridPos, Block /*before*/, Block /*after*/)>,
    pub start_change: Option<(StartPlacement /*before*/, StartPlacement /*after*/)>,
}

pub struct EditorState {
    level: LevelData,
    undo_stack: Vec<EditOp>,
    redo_stack: Vec<EditOp>,
    dirty: bool,
}
impl EditorState {
    pub fn apply_stroke(&mut self, cells: ...) { ... }  // push undo, clear redo
    pub fn undo(&mut self) { ... }
    pub fn redo(&mut self) { ... }
}
```

- ドラッグ中は一時バッファに溜め、ボタンを離した時点で 1 op として確定する
  (同一セルの重複塗りは1エントリに畳む)。
- キー: Ctrl+Z / Ctrl+Y(egui 側で入力を取る)。
- undo_stack の上限は 256 op(超えたら古いものから捨てる)。

## 実装ステップ

### Step 1: プロジェクト形式と移行

- `src/project.rs` 新設(load/save、LimitsConfig の serde 化含む)。
- `assets/projects/sample/` を作成し、test_level.ron を level00.ron として移行。
  project.ron はデフォルト LimitsConfig で書く。
- main.rs をプロジェクト読込に切替。`DEEPGRID_DEBUG_SHOT` シーンが
  従来どおり全部通ることを確認(props のスクショで KayKit モデルも出る)。

### Step 2: ラウンドトリップのユニットテスト

- `cargo test` がホストで通るようにする(`CARGO_TARGET_DIR` 指定。
  GUI 不要のテストのみ: project.rs と editor コア)。
- テスト: sample プロジェクトを load→save→load して Level/start が一致する。
  文字マップの全ブロック種(`# . ~ ^ % H 1 2 < > n v`)を往復させる。

### Step 3: 起動モードと bevy_egui

- CLI パース(`--edit`, `--project <dir>`)。
- bevy_egui 導入。エディットモード時は EguiPlugin +エディター系システムのみ、
  プレイモード時は従来システムのみを App に登録する(実行時分岐は
  `app.add_systems` の登録段階で行い、システム内での毎フレーム分岐を避ける)。

### Step 4: エディターUI

- 上記「エディターの仕様」のレイアウトを実装。
- 編集はすべて `EditorState` のコア API 経由(UI コードにデータ変更を書かない)。

### Step 5: Undo/Redo と保存

- ストローク確定、Ctrl+Z/Y、Save ボタン(+Ctrl+S)。
- 保存→プレイモードで起動し直して編集が反映されていることを確認。

### Step 6: 検証シーン

- `DEEPGRID_DEBUG_SHOT=editor` : エディットモードで sample を開き、
  中央グリッドが描画された状態でスクリーンショットして終了。
  (エディターは操作スクリプトまでは不要。画面が出ることの確認でよい)
- 既存シーン(`1|fall|ladder|door|props`)がプロジェクト形式移行後も
  そのまま通ること。

## 受け入れ基準

1. `./docker/deepgrid-build.sh` 完走、ホスト `cargo clippy` 警告なし、
   ホスト `cargo test` 全通過。
2. `DEEPGRID_DEBUG_SHOT=1|fall|ladder|door|props` が移行後も全て撮影でき、
   内容が plan2 時点と同等(mtime 確認込み)。
3. `DEEPGRID_DEBUG_SHOT=editor` でエディター画面のスクリーンショットが撮れ、
   グリッド・パレット・上部バーが写っている。
4. 手動確認(人間が実施): `--edit` で起動→ブロックを塗る→Undo/Redo が
   期待どおり→Save→プレイモードで起動して編集が反映されている。
5. サンプルプロジェクトがコミットされており、リポジトリを clone した状態で
   上記がすべて再現できる。

## 実装上の注意

- 上限値のハードコード禁止(LimitsConfig は project.ron が正)。
  マップサイズがエディターの描画都合の定数に縛られないこと。
- ラウンドトリップで**コメントは消えてよい**(RON のコメント保持はしない)。
  test_level.ron にあった説明コメントのうち検証シーンの前提
  (fall/ladder/door の座標)は、level00.ron 冒頭に再記載しておくこと。
- `save_level` は書き込み失敗時にパニックせずエラーをUIに表示する
  (ステータスバーで十分)。
- egui は即時モード。1フレームに1回しか状態を触らないこと
  (ドラッグ処理で同一フレームに複数セルを塗るのは可)。
- `ClusterConfig::Single`(プレイモードのライト対策)はエディットモードでは
  不要(3Dカメラ自体を出さない)。
- bevy_egui 0.33 と Bevy 0.15 の組で使うこと(mycity-simulator と同じ)。
