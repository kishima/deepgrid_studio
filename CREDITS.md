# CREDITS — 利用素材の記録

本プロジェクトで利用する素材(外部・自作生成とも)の全件記録。
運用ルールは [CLAUDE.md](CLAUDE.md)「素材の記録ルール」を参照。

## テクスチャ

すべて [ambientCG](https://ambientcg.com/) の CC0 1.0
(パブリックドメイン相当。帰属表記不要・商用利用可 —
https://docs.ambientcg.com/license/ )。
1K解像度PNG版のColorマップのみ使用(Normal/Roughness等は未使用。
導入する場合はタンジェント生成が必要な点に注意)。

| 素材 | 入手元 | ライセンス | ファイル | 用途 | 導入日 |
| --- | --- | --- | --- | --- | --- |
| Bricks 066 | https://ambientcg.com/view?id=Bricks066 | CC0 1.0 | assets/textures/wall_bricks066_color.png | 壁ブロック | 2026-07-12 |
| Paving Stones 119 | https://ambientcg.com/view?id=PavingStones119 | CC0 1.0 | assets/textures/floor_pavingstones119_color.png | 床 | 2026-07-12 |
| Rock 058 | https://ambientcg.com/view?id=Rock058 | CC0 1.0 | assets/textures/ceiling_rock058_color.png | 天井 | 2026-07-12 |

## 3Dモデル

すべて Kay Lousberg 氏(https://www.kaylousberg.com)の KayKit シリーズ。
CC0 1.0(各パック同梱の LICENSE.txt に明記。帰属表記不要・商用利用可)。
glTF Binary(.glb)、テクスチャ内蔵。GitHub のリリース版から取得。

| 素材 | 入手元 | ライセンス | ファイル | 用途 | 導入日 |
| --- | --- | --- | --- | --- | --- |
| KayKit Character Pack: Skeletons 1.0 — Skeleton_Minion | https://github.com/KayKit-Game-Assets/KayKit-Character-Pack-Skeletons-1.0 | CC0 1.0 | assets/models/enemies/skeleton_minion.glb | エネミー表示(アニメーション付き) | 2026-07-12 |
| KayKit Character Pack: Skeletons 1.0 — Skeleton_Warrior | 同上 | CC0 1.0 | assets/models/enemies/skeleton_warrior.glb | エネミー表示(アニメーション付き) | 2026-07-12 |
| KayKit Dungeon Remastered 1.0 — chest | https://github.com/KayKit-Game-Assets/KayKit-Dungeon-Remastered-1.0 | CC0 1.0 | assets/models/props/chest.glb | アイテム表示(宝箱) | 2026-07-12 |
| KayKit Dungeon Remastered 1.0 — barrel_small | 同上 | CC0 1.0 | assets/models/props/barrel_small.glb | アイテム表示(樽) | 2026-07-12 |
| KayKit Adventurers 1.0 — Knight | https://github.com/KayKit-Game-Assets/KayKit-Character-Pack-Adventures-1.0 | CC0 1.0 | assets/models/party/knight.glb | パーティキャラ(戦士)・ポートレート生成元 | 2026-07-13 |
| KayKit Adventurers 1.0 — Mage | 同上 | CC0 1.0 | assets/models/party/mage.glb | パーティキャラ(魔法使い)・ポートレート生成元 | 2026-07-13 |
| KayKit Adventurers 1.0 — Rogue | 同上 | CC0 1.0 | assets/models/party/rogue.glb | パーティキャラ(盗賊)・ポートレート生成元 | 2026-07-13 |
| KayKit Adventurers 1.0 — Rogue_Hooded | 同上 | CC0 1.0 | assets/models/party/rogue_hooded.glb | パーティキャラ(僧侶役のフード姿)・ポートレート生成元 | 2026-07-13 |
| KayKit Adventurers 1.0 — Barbarian | 同上 | CC0 1.0 | assets/models/party/barbarian.glb | パーティキャラ(蛮族)・ポートレート生成元 | 2026-07-13 |

## フォント

| 素材 | 入手元 | ライセンス | ファイル | 用途 | 導入日 |
| --- | --- | --- | --- | --- | --- |
| PixelMplus12 Regular/Bold(M+ FONTS PROJECT / itouhiro) | https://github.com/itouhiro/PixelMplus (v1.0.0) | M+ FONT LICENSE(使用・改変・再配布無制限、商用可) | assets/fonts/PixelMplus12-Regular.ttf, assets/fonts/PixelMplus12-Bold.ttf | ゲーム内UIの日本語テキスト(8bit風ピクセルフォント) | 2026-07-13 |

## ポートレート画像(パブリックドメインの版画から加工)

assets/projects/sample/portraits/ の5枚。Wikimedia Commons から取得した
パブリックドメイン(またはCC0指定)の版画をグレースケールのバストアップ
(256×256)に切り出したもの。**再生成スクリプト**: scripts/fetch_portraits.py
(取得元URL・切り出しパラメーターはスクリプトが正)。

| ファイル | 原画 | 入手元 | ライセンス | 導入日 |
| --- | --- | --- | --- | --- |
| portraits/knight.png | アルブレヒト・デューラー「騎士と死と悪魔」(1513) | https://commons.wikimedia.org/wiki/File:Knight,_Death_and_the_Devil_MET_DP159049.jpg | CC0(MET提供) | 2026-07-14 |
| portraits/mage.png | レンブラント「書斎の学者(ファウスト)」(c.1652) | https://commons.wikimedia.org/wiki/File:A_Scholar_in_His_Study_('Faust')_MET_DP814791.jpg | CC0(MET提供) | 2026-07-14 |
| portraits/priest.png | ギュスターヴ・ドレ「バルク」(聖書挿絵、1866) | https://commons.wikimedia.org/wiki/File:125.Baruch.jpg | パブリックドメイン | 2026-07-14 |
| portraits/rogue.png | ギュスターヴ・ドレ「ユダの接吻」(聖書挿絵、1866) | https://commons.wikimedia.org/wiki/File:Gustave_Doré_-_The_Holy_Bible_-_Plate_CXLI,_The_Judas_Kiss.jpg | パブリックドメイン | 2026-07-14 |
| portraits/barbarian.png | ギュスターヴ・ドレ「獅子を裂くサムソン」(聖書挿絵、1866) | https://commons.wikimedia.org/wiki/File:060.Samson_Slays_a_Lion.jpg | パブリックドメイン | 2026-07-14 |

## 自作・生成素材

(まだなし。追加時は生成スクリプトのパスと生成手順をここに記録する。
※ポートレートの加工スクリプトは上記の節を参照)
