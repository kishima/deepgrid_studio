# assets/models

ダンジョン内に表示する3Dモデル(glTF Binary、テクスチャ内蔵)。
全件 Kay Lousberg 氏の KayKit シリーズ(CC0 1.0)。
詳細・全件一覧はリポジトリ直下の [CREDITS.md](../../CREDITS.md) を参照。

- `enemies/skeleton_minion.glb` — KayKit Character Pack: Skeletons 1.0。リグ+95アニメーション入り。
  plan6 のモンスター表示元(monsters.ron から**名前指定**でアニメを引く:
  "Idle" "Walking_A" "1H_Melee_Attack_Slice_Diagonal" "Hit_A" "Death_A" 等)。
  サンプルの4種(minion/warrior/rogue/mage)はこの2モデルを流用している
  (skeleton_mage/rogue の追加取得は見送り)。
- `enemies/skeleton_warrior.glb` — 同上
- `props/chest.glb` — KayKit Dungeon Remastered 1.0
- `props/barrel_small.glb` — 同上(配布名 barrel_small.gltf.glb を改名)
- `party/knight.glb` `party/mage.glb` `party/rogue.glb` `party/rogue_hooded.glb` `party/barbarian.glb`
  — KayKit Adventurers 1.0。パーティキャラの見た目+ポートレート生成元。
  リグ+76アニメーション入り(Idle はインデックス36)
