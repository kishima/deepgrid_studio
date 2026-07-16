# DeepGrid Studio

PC98 の『だんだんダンジョン』(ダンジョンマスター風のリアルタイム3D
ダンジョンRPG作成ツール)をオマージュした、Rust + [Bevy](https://bevyengine.org/) 0.15
製のプロジェクト。2つの顔を持つ:

- **ランタイム(プレイヤー)** — 作られたゲームを遊ぶ一人称グリッド式3Dダンジョン
- **スタジオ(エディター)** — ゲームを作る編集系(マップ/キャラ/アイテム/
  モンスター/魔法/イベント/デモ)

仕様・計画の詳細は [docs/project.md](docs/project.md) / [docs/roadmap.md](docs/roadmap.md) /
各 `docs/planN.md` を参照。開発方針は [CLAUDE.md](CLAUDE.md) にまとめている。
この README は「遊ぶ人」「作る人」「開発者」の3部構成。

---

## 遊ぶ人向け

### 起動

```sh
./docker/deepgrid-run.sh                    # サンプルゲーム(WSL2 + docker)
./docker/deepgrid-run.sh --project <dir>    # 別のゲームを読み込む
./docker/deepgrid-run.sh --load 1           # セーブスロット1から直接再開
./scripts/deepgrid-run-win.sh               # Windows ネイティブ(GPU)で遊ぶ
```

起動するとタイトル画面が開く(plan11)。**はじめから**(OPデモ→開始)、
**つづきから**(スロット1〜3)、**設定**(音量・足音・ゲーム速度・キー割当。
即時反映され `user_settings.ron` に保存)、**クレジット**(CREDITS.md を表示)、
**ゲームを選ぶ**(隣接プロジェクトへ切り替え)、**終了**。
配布版(`deepgrid.ron` の `play_only`)ではエディターとゲーム切り替えは使えない。

### 操作(プレイモード)

グリッド単位の移動と90度単位の視点回転(移動・回転は 0.25 秒で滑らかに補間)。

| キー | 動作 |
| --- | --- |
| `W` / `S` | 前進 / 後退 |
| `A` / `D` | 左 / 右ストレイフ(平行移動) |
| `Q` / `E` | 左 / 右90度回転 |
| `R` / `F` | はしごを上る / 下りる |
| `Space` | 正面のドア開閉・調べる・攻撃 |
| `G` / `B` | 拾う / 防御 |
| `Tab` / `I` / `M` | データ画面(持ち物・装備・魔法・セーブ) |
| `O` | プレイ中のキー割当変更 |

- 足場の無いマスに入ると落下し、落下フロア数に応じてダメージを受ける。
- 右のステータスウインドーに4人のパーティ(顔ポートレート+HP=青/MP=赤/
  集中力=緑のバー)、下のメッセージウインドーにイベントログが出る。
- ゲーム内のエンディングを迎えるとタイトルに戻る(パーティは初期化)。
- ウィンドウサイズは可変。UI は追従する。

---

## 作る人向け

### エディター

```sh
./docker/deepgrid-run.sh --edit             # エディターを開く
```

- タブ: マップ / キャラ / アイテム / モンスター / 魔法 / イベント / デモ / 設定。
  いずれも「左に一覧・右に詳細」。Undo / Redo、`Save All`(1世代 `*.ron.bak`)。
- マップ: 左のパレットからブロックを選びドラッグで描画。右クリックで開始位置。
  3Dエディットモードでダンジョン内を歩きながら配置もできる。
- 設定タブ: プロジェクト名・**作者・説明**(タイトル画面に表示、v8)・上限値
  (LimitsConfig)・ゲームルール・フラグ初期値・グラフィック差し替え(override/)。

### プロジェクト形式

1つのゲームは1ディレクトリ(詳細は [src/project.rs](src/project.rs)):

```text
<project>/
├── project.ron        # メタデータ(name/author/description) + LimitsConfig + party
├── characters.ron     # 登録キャラクター
├── items.ron / monsters.ron / magics.ron / demos.ron
├── levels/levelNN.ron # マップ(フロア積層)
└── override/          # 組み込み素材の差し替え(同じ相対パスを置く)
```

- 数量上限はすべて `LimitsConfig` 経由で扱い、ハードコードしない
  (project.md「上限値の扱い」)。旧バージョン(v1〜v7)も後方互換で読める。
- デモ id `"op"` は「はじめから」で自動再生、id `"ed"` は再生後に
  タイトルへ戻る(ゲームクリア)。

### 配布

```sh
./docker/deepgrid-build.sh                             # リリースビルド
./scripts/export-game.sh assets/projects/<name> <out>  # 配布フォルダーを生成
```

出力先にはバイナリ(+リポジトリ直下に `deepgrid_studio.exe` があれば同梱)、
`assets/`(対象プロジェクトのみ。saves/ と `*.ron.bak` は除外)、CREDITS.md、
遊ぶ人向け README、そして `deepgrid.ron`(`play_only: true` +プロジェクト固定)
が入り、そのフォルダー単体で起動できる。zip 化は手動で。

---

## 開発者向け

### 動作環境

- WSL2 (Ubuntu) + WSLg(GUI は Windows デスクトップに表示)
- Docker(ビルド・実行の標準経路 = 検証基準。ソフトウェア Vulkan / lavapipe)
- ビルドイメージ `gaia-maker-build`(無ければ
  [docker/Dockerfile](docker/Dockerfile) から自動生成)

> **ホストでの `cargo run` は不可**(`libxkbcommon-x11-0` が無く起動時に panic)。
> 実行は必ず docker(または Windows ネイティブ)経由で行う。
> ホストの `cargo check` / `clippy` / `test` は可(`target/` が root 所有のため
> `CARGO_TARGET_DIR` を別ディレクトリに指定すること)。

### ビルド・実行

```sh
./docker/deepgrid-build.sh          # release ビルド(標準)
./docker/deepgrid-build.sh debug    # debug ビルド
./docker/deepgrid-run.sh [debug] [--edit|--project <dir>|--load <slot>]
./target/release/deepgrid_studio --help   # 全オプションと環境変数の一覧
```

第1引数が `release` / `debug` のときだけビルドモード指定、それ以外はバイナリへ
そのまま渡される。`--project` 省略時は `assets/projects/sample`。

lavapipe はフィルレート律速のため、docker の**対話プレイのみ**
`DEEPGRID_WINDOW=960x540` が既定になる(検証ショット・autotest は 1280x720 の
まま)。`DEEPGRID_WINDOW=WxH` で任意に上書きできる(plan11 計測値は
[docs/plan11.md](docs/plan11.md))。

### Windows ネイティブ GPU 実行

docker 経路は lavapipe(CPU レンダリング)。GPU で快適に動かす・性能を測る
場合は Windows ネイティブ .exe をクロスビルドして interop で起動する:

```sh
# 一回だけの導入(sudo が必要)
~/.cargo/bin/rustup target add x86_64-pc-windows-gnu
sudo apt install gcc-mingw-w64-x86-64

./scripts/deepgrid-run-win.sh       # ビルド → exe をリポジトリ直下へ → 起動
```

起動ログの `AdapterInfo` が NVIDIA + Vulkan であることを確認する。
`DEEPGRID_*` / `WGPU_*` は WSLENV で橋渡しされる(env を増やしたら
run-win.sh と docker/deepgrid-run.sh の両方に追加)。

### 検証

**`./scripts/verify-all.sh` が全項目の基準**: clippy → cargo test →
docker build → autotest → 全シーン撮影(mtime 検査)→ export 検証を順に実行し、
PASS/FAIL 一覧と終了コードを返す。

個別に回す場合:

```sh
CARGO_TARGET_DIR=/tmp/deepgrid-check ~/.cargo/bin/cargo clippy --all-targets
CARGO_TARGET_DIR=/tmp/deepgrid-check ~/.cargo/bin/cargo test
DEEPGRID_AUTOTEST=1 ./docker/deepgrid-run.sh          # 無人受け入れテスト(49ステップ)
DEEPGRID_DEBUG_SHOT=<scene> ./docker/deepgrid-run.sh  # debug-shot.png を出力し終了
```

プレイ側シーン: `1`(メイン画面) `fall` `ladder` `door` `monster` `magic`
`light` `potion` `plate` `warp` `stairs` `hole` `combat` `items` `pickup`
`data` `liquid` `demo` `override` `title`(タイトル画面、plan11)。

エディター側シーン: `editor`(=`editor-map`) `editor-chars` `editor-items`
`editor-monsters` `editor-magics` `editor-events` `editor-demos`
`editor-settings`(以上 egui 撮影)、`editor-3d`(Bevy Screenshot)。

シーン一覧の機械可読な正は `scripts/verify-all.sh` 冒頭の配列
(この README はそれを写したもの)。生成物の mtime が今回の実行時刻で
あることを確認してから使うこと。

### パフォーマンス計測

```sh
python3 scripts/gen_stress_project.py     # 上限いっぱいの計測用プロジェクト生成
DEEPGRID_PERF=10 ./docker/deepgrid-run.sh --project assets/projects/stress
DEEPGRID_PERF=10 ./scripts/deepgrid-run-win.sh --project assets/projects/stress
```

起動後ウォームアップを除いた平均/最悪フレーム時間を標準出力に出して終了する。
目標: lavapipe で 20fps 以上、Windows GPU で 60fps(plan11。計測値は
[docs/plan11.md](docs/plan11.md) に記録)。

## ライセンス / 素材

- コード: GPL-3.0(`Cargo.toml`)。
- 利用素材(テクスチャ・3Dモデル・フォント・音源)は外部・自作生成を問わず
  全件 [CREDITS.md](CREDITS.md) と各 `assets/*/README.md` に記録している
  (運用ルールは [CLAUDE.md](CLAUDE.md)「素材の記録ルール」)。
  ランタイムのタイトル画面「クレジット」からも閲覧できる。
