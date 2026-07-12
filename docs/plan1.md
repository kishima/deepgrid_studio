# plan1: プロジェクト基盤構築 + 歩ける3Dダンジョンのプロトタイプ

## この文書について

DeepGrid Studio の最初の実装計画書。実装者(AI/人)がこの文書と
[project.md](project.md) だけを読めば着手できるように書いてある。
不明点があれば project.md を正とし、この文書とプロジェクト仕様が
矛盾する場合は project.md を優先すること。

## 背景(要約)

DeepGrid Studio は、PC98 の「だんだんダンジョン」(ダンジョンマスター風の
リアルタイム3DダンジョンRPG作成ツール)のオマージュ。Rust + Bevy で開発する。
詳細仕様は [project.md](project.md) を参照。

plan1 のゴールは **「グリッド式ダンジョンの中を一人称視点で滑らかに歩き回れる
最小のゲーム画面」を、再現可能なビルド・実行環境つきで動かすこと**。
エディターやモンスター・アイテム・戦闘などは plan2 以降で扱う。

## スコープ

### やること

1. Cargo プロジェクトの雛形作成
2. docker ベースのビルド・実行環境(スクリプト含む)
3. ダンジョンデータの最小データモデル(設定可能な上限値の骨格を含む)
4. テスト用マップ(RONファイル)の読み込み
5. 一人称グリッド移動(滑らか補間つき)ができる3D表示
6. 検証用スクリーンショット機構(`DEEPGRID_DEBUG_SHOT=1`)

### やらないこと(plan2以降)

- エディター画面(マップエディター、キャラ/アイテム/モンスター/魔法エディター)
- モンスター、アイテム、魔法、イベント、戦闘、パーティ管理
- メイン画面のウインドー群(ステータス、アクションアイコン等)。plan1 は3Dビューのみ
- 高さ・落下の実装(データモデルにフロア層は持たせるが、移動は単一フロア内のみ)
- BGM・効果音
- Windows ネイティブ GPU 実行スクリプト(構成だけ project.md に記載済み。必要になったら別plan)

## 開発環境の前提(重要)

ホストは WSL2 (Ubuntu) + WSLg。以下の制約を必ず守ること。

- **実行は必ず docker 経由**。ホストには `libxkbcommon-x11-0` が無く、
  ホストの `cargo run` は起動直後に panic して失敗する。
- ホストの cargo(`~/.cargo/bin/cargo`)は `cargo check` / `cargo clippy` /
  `cargo fmt` にのみ使用可。ただしリポジトリの `target/` は docker が生成した
  root 所有になるため、**必ず `CARGO_TARGET_DIR` を書き込み可能な別ディレクトリに
  指定する**こと(例: `CARGO_TARGET_DIR=/tmp/deepgrid-check cargo check`)。
- docker イメージは `gaia-maker-build` を使う(隣接プロジェクト
  `/home/kishima/game-dev/mycity-simulator` と共用)。イメージが無ければ
  本リポジトリの `docker/Dockerfile` からビルドする。Dockerfile は
  `mycity-simulator/docker/Dockerfile` をそのままコピーしてよい。
- cargo レジストリキャッシュは docker ボリューム `gaia-cargo-registry` を共用する。

参考実装として `/home/kishima/game-dev/mycity-simulator` の
`docker/mycity-build.sh`, `docker/mycity-run.sh`, `Cargo.toml`, `CLAUDE.md` を
必ず一読すること。ビルド・実行スクリプトはこれらの改名・微修正で足りる。

## 成果物一覧

```
deepgrid_studio/
├── Cargo.toml
├── .gitignore                 # /target, *.exe, debug-shot.png など
├── docker/
│   ├── Dockerfile             # mycity-simulator からコピー
│   ├── deepgrid-build.sh      # ビルド(release既定、引数 debug で debug)
│   └── deepgrid-run.sh        # WSLg 経由で実行
├── assets/
│   └── maps/
│       └── test_level.ron     # テスト用マップ
├── src/
│   ├── main.rs                # Bevy App 組み立てのみ
│   ├── config.rs              # LimitsConfig(上限値設定)
│   ├── dungeon/
│   │   ├── mod.rs
│   │   ├── block.rs           # Block enum ほかデータモデル
│   │   ├── level.rs           # Level / Floor / 座標型
│   │   └── loader.rs          # RON からの読み込み
│   ├── player/
│   │   ├── mod.rs
│   │   └── movement.rs        # グリッド移動 + 補間 + 入力バッファ
│   ├── render/
│   │   ├── mod.rs
│   │   └── dungeon_mesh.rs    # フロア → 3Dメッシュ生成
│   └── debug_shot.rs          # DEEPGRID_DEBUG_SHOT 対応
└── docs/
    ├── project.md             # 既存
    └── plan1.md               # 本文書
```

## 実装ステップ

### Step 1: プロジェクト雛形と docker 環境

1. `Cargo.toml` を作成する。依存は mycity-simulator の Cargo.toml に合わせる:
   - `bevy = "0.15"`(default-features無効、featureは mycity-simulator と同じセットで開始してよい。
     音声は plan1 では使わないので audio 系 feature は不要)
   - `serde`(derive付き), `ron = "0.8"`
   - `bevy_egui = "0.33"` は plan1 では未使用なので**入れない**(エディター実装時に追加)
   - edition 2024、`[profile.dev] opt-level = 1`、`[profile.release] strip = true`
2. `docker/Dockerfile` を mycity-simulator からコピー。
3. `docker/deepgrid-build.sh` / `docker/deepgrid-run.sh` を
   `mycity-build.sh` / `mycity-run.sh` を元に作成。変更点:
   - バイナリ名を `deepgrid_studio` に
   - 転送する環境変数を `DEEPGRID_DEBUG_SHOT` に(`MYCITY_*` は不要)
4. 動作確認: 空の Bevy App(ウインドウを開くだけ)が
   `./docker/deepgrid-build.sh && ./docker/deepgrid-run.sh` で
   Windows デスクトップに表示されること。

### Step 2: データモデル

`src/config.rs`:

```rust
/// 数量上限の設定。オリジナル準拠の初期値を持つが、すべて可変
/// (project.md「上限値の扱い」参照)。plan1 で使うのは一部のみだが
/// 骨格として全項目を定義しておく。
#[derive(Resource, Serialize, Deserialize, Clone)]
pub struct LimitsConfig {
    pub max_levels: usize,            // 14
    pub floors_per_level: usize,      // 5
    pub floor_width: usize,           // 40
    pub floor_height: usize,          // 40
    // ... project.md の表の項目を同様に
}
impl Default for LimitsConfig { /* オリジナル準拠の初期値 */ }
```

`src/dungeon/block.rs`:

```rust
/// ブロックの基本属性(project.md「ダンジョン構造の仕様」)。
/// plan1 では Wall / Empty のみ移動判定に使う。液体系は描画も判定も plan2 以降。
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
pub enum Block {
    Wall,
    Empty,
    Water,
    Fire,
    Poison,
}
```

`src/dungeon/level.rs`:

- `Floor`: `Vec<Block>`(width × height、固定長配列にしないこと)+ width/height
- `Level`: `Vec<Floor>`(下から上へ)
- グリッド座標型 `GridPos { x: i32, y: i32, floor: usize }` と
  方角 `Facing { North, East, South, West }`(90度回転メソッド付き)

### Step 3: マップ読み込み

- `assets/maps/test_level.ron` に 1フロア分のテストマップを RON で記述する。
  サイズは 40×40(既定値)。外周は壁、内部に通路が数本、部屋が2〜3個ある
  程度の手書きマップでよい。プレイヤー開始位置と向きもファイルに含める。
- `src/dungeon/loader.rs` で起動時に読み込み、`Resource` として保持する。
  読み込み失敗時は panic でよい(plan1 ではエラーUIは不要)。

### Step 4: 3D表示

`src/render/dungeon_mesh.rs`:

- フロアデータから床・天井・壁のメッシュを生成する。
  1ブロック = 1.0 × 1.0 × 1.0(高さも1.0)のワールド座標とする。
- plan1 の見た目は単色マテリアル(壁・床・天井で色を変える)+
  `PointLight` をプレイヤーに追随させる、で十分。テクスチャは不要。
- 実装を単純にするため、壁ブロックごとに Cuboid を1個置く方式でよい
  (40×40 なら性能上問題ない。面カリング等の最適化はしない)。

### Step 5: グリッド移動

`src/player/movement.rs`(project.md「UIの方針」参照):

- プレイヤーの論理状態は `GridPos` + `Facing`。カメラは論理状態に向かって
  Transform を補間する。
- キー割り当て: `W`前進 / `S`後退 / `A`左ストレイフ / `D`右ストレイフ /
  `Q`左90度回転 / `E`右90度回転。
- 1歩・90度回転は **0.25秒のイージング付きアニメーション**
  (ease-in-out。線形でも可だが定数は分離しておく)。
- **移動判定はマス単位**: 移動先が `Block::Wall` なら移動しない
  (アニメーションも開始しない)。それ以外のブロックは通行可。
- **入力バッファリング**: アニメーション中に押されたキーは1つだけ保持し、
  アニメーション完了時に即座に次の移動を開始する。キー押しっぱなしで
  歩みが途切れないこと。
- カメラの目線高さは床から 0.5(ブロック中央)程度。FOV は 70〜90度で調整。

### Step 6: 検証用スクリーンショット

`src/debug_shot.rs`:

- 環境変数 `DEEPGRID_DEBUG_SHOT=1` が設定されているとき:
  数フレーム描画を待ってから(レンダリング安定のため30フレーム程度)、
  スクリーンショットを `debug-shot.png` としてリポジトリ直下に保存し、
  `AppExit` で自動終了する。
- Bevy 0.15 の `bevy::render::view::screenshot`(`Screenshot` /
  `save_to_disk`)を使う。参考: mycity-simulator の同等機能の実装
  (`grep -r DEBUG_SHOT /home/kishima/game-dev/mycity-simulator/src`)。

## 受け入れ基準

すべて満たすこと:

1. `./docker/deepgrid-build.sh` がエラーなく完走する。
2. `./docker/deepgrid-run.sh` でウインドウが開き、一人称視点のダンジョンが表示される。
3. WASD/QE でグリッド移動・回転ができ、動きが滑らかに補間される。
   壁にはめり込めない。キー押しっぱなしで連続歩行できる。
4. `DEEPGRID_DEBUG_SHOT=1 ./docker/deepgrid-run.sh` で `debug-shot.png` が
   生成されて自動終了し、画像にダンジョンの壁・床・天井が写っている。
   **生成物の mtime が今回の実行時刻であることを確認する**こと
   (過去の実行の残骸と取り違えない)。
5. ホストで `CARGO_TARGET_DIR=/tmp/deepgrid-check cargo clippy` が警告なしで通る
   (許容できる警告は `#[allow]` の理由コメント付きで抑制)。

## 検証手順(実装者が最後に実行して結果を報告すること)

```sh
cd /home/kishima/game-dev/deepgrid_studio
./docker/deepgrid-build.sh
DEEPGRID_DEBUG_SHOT=1 ./docker/deepgrid-run.sh
ls -l --time-style=full-iso debug-shot.png   # mtime が今回実行時刻か確認
CARGO_TARGET_DIR=/tmp/deepgrid-check ~/.cargo/bin/cargo clippy
```

対話的な操作確認(WASD/QE移動)は人間が行うため、実装者は上記の
自動検証まで通した状態で引き渡すこと。debug-shot.png は目視確認用に
そのまま残しておくこと。

## 実装上の注意

- モジュール分割は成果物一覧の構成に従う。`main.rs` にロジックを書かない。
- 上限値(マップサイズ等)を定数でハードコードしない。必ず `LimitsConfig`
  経由で参照する(plan1 で実際に可変にするのは floor_width / floor_height のみでよいが、
  参照経路だけは最初から LimitsConfig に通しておく)。
- Bevy のバージョンは 0.15 に固定する。0.16 以降の API(必須コンポーネント形式の
  変更等)と混同しないこと。
- コミットは論理的な単位(Step ごと程度)で分ける。
