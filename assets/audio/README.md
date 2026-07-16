# assets/audio — 音源の由来

plan10 で導入。全件 CC0(パブリックドメイン相当)。詳細な入手元 URL・
ライセンスは リポジトリ直下の [CREDITS.md](../../CREDITS.md) を参照。
アーカイブ原本はコミットしない(CREDITS.md の URL から再取得可能)。

## se/ — 効果音(Kenney.nl、CC0)

| ファイル | 用途 | 元ファイル | パック |
|---|---|---|---|
| footstep.ogg | 足音 | footstep_concrete_000.ogg | Impact Sounds |
| door_open.ogg | ドア開 | doorOpen_1.ogg | RPG Audio |
| door_close.ogg | ドア閉 | doorClose_1.ogg | RPG Audio |
| melee_hit.ogg | 攻撃ヒット | knifeSlice.ogg | RPG Audio |
| spell_cast.ogg | 魔法詠唱 | phaserUp2.ogg | Digital Audio |
| magic_impact.ogg | 光弾着弾 | impactBell_heavy_002.ogg | Impact Sounds |
| fall_thud.ogg | 落下着地 | impactSoft_heavy_001.ogg | Impact Sounds |
| item_pickup.ogg | 拾う | handleCoins.ogg | RPG Audio |
| level_up.ogg | レベルアップ | powerUp5.ogg | Digital Audio |
| switch_click.ogg | スイッチ/しかけ床 | metalClick.ogg | RPG Audio |
| warp.ogg | ワープ | phaseJump1.ogg | Digital Audio |

## bgm/ — BGM(OpenGameArt.org、CC0)

| ファイル | 曲名 | 作者 | 備考 |
|---|---|---|---|
| bgm_dungeon1.ogg | Dungeon Ambience | yd | |
| bgm_dungeon2.ogg | Wander in a dungeon | Kosmo The Cat | |
| bgm_battle.ogg | Battle Theme A | cynicmusic | MP3→OGG 変換 |
| bgm_town.ogg | Calming RPG Town Theme | Destin715 | MP3→OGG 変換 |
| bgm_mysterious.ogg | Mysterious Ambience (song21) | cynicmusic | 複数ライセンスから CC0 を選択。MP3→OGG 変換 |
| bgm_tension.ogg | Tension Theme | Umplix | WAV→OGG 変換 |
| bgm_ending.ogg | Victory Theme for RPG | cynicmusic | MP3→OGG 変換 |

BGM のファイル名は `LevelData.bgm` / デモの `bgm` にファイル名だけで指定する
(例: `bgm: "bgm_dungeon1.ogg"`)。
