# DeepGrid Studio

PC98 の『だんだんダンジョン』(ダンジョンマスター風のリアルタイム3D
ダンジョンRPG作成ツール)をオマージュした、Rust + [Bevy](https://bevyengine.org/) 0.15
製のプロジェクト。2つの顔を持つ:

- **ランタイム(プレイヤー)** — 作られたゲームを遊ぶ一人称グリッド式3Dダンジョン
- **スタジオ(エディター)** — ゲームを作る編集系(現状はマップエディター)

仕様・計画の詳細は [docs/project.md](docs/project.md) / [docs/roadmap.md](docs/roadmap.md) /
各 `docs/planN.md` を参照。開発方針は [CLAUDE.md](CLAUDE.md) にまとめている。

## 動作環境

- WSL2 (Ubuntu) + WSLg(GUI は Windows デスクトップに表示)
- Docker(ビルド・実行の標準経路。ソフトウェア Vulkan / lavapipe を使う)
- ビルドイメージ `gaia-maker-build`(無ければビルドスクリプトが
  [docker/Dockerfile](docker/Dockerfile) から自動生成)

> **ホストでの `cargo run` は不可**(`libxkbcommon-x11-0` が無く起動時に panic)。
> 実行は必ず docker(または Windows ネイティブ)経由で行う。
> ホストの `cargo check` / `clippy` / `fmt` は可(`target/` が root 所有のため
> `CARGO_TARGET_DIR` を別ディレクトリに指定すること)。

## ビルド

```sh
./docker/deepgrid-build.sh          # release ビルド(標準)
./docker/deepgrid-build.sh debug    # debug ビルド
```

`target/` はホスト側に生成される(所有者は root になる)。

## 実行

```sh
./docker/deepgrid-run.sh                    # プレイモード(サンプルプロジェクト)
./docker/deepgrid-run.sh debug              # debug ビルドを実行
./docker/deepgrid-run.sh --edit             # マップエディターを開く
./docker/deepgrid-run.sh --project <dir>    # 別プロジェクトを読み込む
./docker/deepgrid-run.sh --load 1           # セーブスロット1から再開(plan10)
```

- 第1引数が `release` / `debug` のときだけビルドモード指定、それ以外の引数
  (`--edit`, `--project <dir>` など)はバイナリへそのまま渡される。
- `--project` を省略すると `assets/projects/sample` を読み込む。

### 操作(プレイモード)

グリッド単位の移動と90度単位の視点回転(移動・回転は 0.25 秒で滑らかに補間)。

| キー | 動作 |
| --- | --- |
| `W` / `S` | 前進 / 後退 |
| `A` / `D` | 左 / 右ストレイフ(平行移動) |
| `Q` / `E` | 左 / 右90度回転 |
| `R` / `F` | はしごを上る / 下りる |
| `Space` | 正面のドアを開閉 |

- 足場の無いマスに入ると落下し、落下フロア数に応じてパーティがダメージを受ける。
- 右のステータスウインドーに4人のパーティ(顔ポートレート+HP=青/MP=赤/
  集中力=緑のバー)、下のメッセージウインドーにイベントログが出る。
- ウィンドウサイズは可変。UI は追従する。

### マップエディター(`--edit`)

- 左のパレットからブロックを選び、マップ上をドラッグして描画。
- 右クリックでプレイヤー開始位置の設定/向きの切り替え。
- `Save` で保存(1世代分 `*.ron.bak` にバックアップ)。Undo / Redo 対応。

## 検証用スクリーンショット

ヘッドレス相当の自動検証。所定シーンを描画して `debug-shot.png` を出力し自動終了する。

```sh
DEEPGRID_DEBUG_SHOT=<scene> ./docker/deepgrid-run.sh
```

プレイ側シーン: `1`(メイン画面) `fall` `ladder` `door` `monster` `magic`
`light` `potion` `plate` `warp` `stairs` `hole` `combat` `items` `pickup`
`data` `liquid` `demo`(デモ再生オーバーレイ、plan10)
`override`(壁テクスチャ差し替え、plan10 — 一時 override を自動生成・撤去)。

エディター側シーン: `editor`(=`editor-map`) `editor-chars` `editor-items`
`editor-monsters` `editor-magics` `editor-events` `editor-demos`(plan10)
`editor-settings`(以上 egui 撮影)、`editor-3d`(3Dエディットモード、
Bevy Screenshot — egui パネルは写らない)。

生成物の mtime が今回の実行時刻であることを確認してから使うこと。

## ホストでの静的チェック

```sh
CARGO_TARGET_DIR=/tmp/deepgrid-check ~/.cargo/bin/cargo clippy --all-targets
CARGO_TARGET_DIR=/tmp/deepgrid-check ~/.cargo/bin/cargo test
```

## プロジェクト形式

1つのゲームは1ディレクトリとして保存される(詳細は [src/project.rs](src/project.rs)):

```text
<project>/
├── project.ron        # メタデータ + LimitsConfig(上限値) + party 編成
├── characters.ron     # 登録キャラクター(Vec<Character>)
└── levels/
    └── levelNN.ron    # マップ(フロア積層)
```

- 数量上限(レベル数・パーティ人数・キャラ登録数など)はすべて `LimitsConfig`
  経由で扱い、ハードコードしない(project.md「上限値の扱い」)。
- 旧 version 1 のプロジェクト(characters / party 無し)も後方互換で読める。

## ライセンス / 素材

- コード: GPL-3.0(`Cargo.toml`)。
- 利用素材(テクスチャ・3Dモデル・フォント)は外部・自作生成を問わず全件
  [CREDITS.md](CREDITS.md) と各 `assets/*/README.md` に記録している
  (運用ルールは [CLAUDE.md](CLAUDE.md)「素材の記録ルール」)。
