# client

> **正本**: [`client/`](../../../client/)（Cargo workspace）  
> **ステータス**: x0014（LAN 2-client presence — sub 別 user_root 3/4）

## レイアウト

| パス | 役割 |
|:---|:---|
| `client/Cargo.toml` | workspace 根（将来 submodule の根） |
| `client/app` | 画面プロセス（パッケージ名 `client`） |
| `client/app/src/system_ui/` | 個人 DashRoot + 公式 Facet（OpenXR / Room を知らない） |
| `client/app/src/surface/` | boot: Desktop (winit) / OpenXR。デバイス差の吸収 |
| `client/app/assets/icons/` | Material Icons（静的同梱） |
| `client/host-server` | 裏プロセスの計算ホスト（Rapier + user_root loco） |
| `client/fixtures/` | Coin_PolygonalMind GLB（CC0） |
| `../protocol` | 共有ワイヤ契約（workspace 外） |

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| Zenoh: Manifest Subscribe / frame デルタ Subscribe / input Publish | ルーター機能（他人の通信の中継は禁止） |
| Manifest の blake3 → assets download（失敗時 fixture）→ BLAKE3 → wgpu | LOD / KTX2 / マテリアル本格 / スキン |
| ローカル entity 表への upsert/remove。Coin=1、user_root=3/4、boxes=10..18 | 署名付き Manifest / world 台帳 |
| Desktop: WASD/Space/C → Intent（look yaw 相対）。クリックで Confined（失敗時 Locked）→ マウスルック。look 中 RMB → Grab（x0013） | 感度 UI / クロスヘア必須 |
| `AUTH_USER=alice\|bob` → 自 entity 3\|4 カメラ；他 root は色付きボックス（x0014）。`ZENOH_CONNECT` LAN | 3人以上・動的 entity 発見・アバター GLB |
| OpenXR: 左右 thumbstick（aim 相対・倒し量・加算）→ Intent | Vive トラックパッド専用 / ホバー |
| OpenXR: `XR_EXT_render_model` 時に実機コントローラ GLB（ルート pose + 埋め込み baseColor + animatable node） | SteamVR パス直読み／PBR（MR/normal 等）／手トラ |
| OpenXR: ダッシュ開→UI レーザー；閉→ワールドレーザー＋ Grab hold（squeeze／Select）（x0011） | Steal／回転追従／Facet 並べ替え／WorldRoot／CompositionLayerQuad |
| auth login → Bearer 付き Room Token → InputEvent（含 grab_entity_id + aim_*） | OSC / Dual Editor / launcher 本実装 |
| host: Rapier 60Hz + `is_finite` + per-`sub` Intent（最大2）+ user_root 3/4 + cuboid×9 + Grab → 30Hz デルタ | Formula / 予測補間 / verifier / 本格 Walk / Steal |
| ESC（Desktop）/ Menu（OpenXR）ダッシュ・Pinned Exit・Button hover・client 終了・host 終了スタブ。Esc でダッシュ開時は look／Grab 解放 | Facet 並べ替え / Context / Primary ツール本体 |
| OpenXR ランタイムあり → XR、なし／`ALCHEMY_FORCE_DESKTOP` → Desktop | WSL2 実 HMD・launcher 本実装 |

## チャネル分離（C-C + E + x0009）

| キー | 頻度 | 用途 |
|:---|:---|:---|
| `…/manifest` | 低（host は 5s 再送で late joiner 補助） | `entity_id → blake3` + `locomotion` |
| `…/frame` | 高（動いたものだけ） | pose upserts / removes（含 user_root） |
| `…/input` | 入力 | Room Token 付き Intent（`move_*` + `buttons`） |
| `…/host_control` | 低（スタブ） | client→host `shutdown`（launcher 前） |

## Desktop バインド（x0009）

| 入力 | 役割 |
|:---|:---|
| Esc | ダッシュ開閉（開いている間は移動 Intent = 0） |
| WASD | 前/左/後/右（固定 look・yaw=0 相対） |
| Space / C | 上昇 / 下降（Noclip・Fly） |
| Y / T / R | Primary / Context / Secondary（キーのみスタブ） |
| マウス | ダッシュ開: UI。閉: LMB で look capture、移動で yaw/pitch（x0012）；look 中は箱を下向き照準（明るい）＋ LMB/RMB/E 長押しで Grab（x0013） |

## OpenXR バインド（x000a / x000b / x000c / x000d / x000e）

| 入力 | 役割 |
|:---|:---|
| Menu | ダッシュ開閉（開いている間は移動 Intent = 0） |
| Select + aim | ダッシュ UI クリック（開中）。閉時はワールドレーザー；squeeze 無し時は Grab も兼ねる |
| squeeze（Grip） | ダッシュ閉・ヒット中に Grab hold 開始／保持（x0011） |
| 左右 thumbstick Y | その手の aim 相対で前後移動（L+R 加算） |
| 左右 thumbstick X | コンフォートターン（L+R 加算。既定 Snap 30° / Smooth 切替可） |
| Settings タブ | max speed・snap 角・smooth 速度の既定表示。Turn モードはタップで切替 |
| コントローラ見た目 | ランタイム `XR_EXT_render_model` GLB（SteamVR 2.12+）。埋め込み baseColor + animatable node（トリガー／スティック等）。拡張欠如時は非表示のみ |

## アセット

- モデル: Coin_PolygonalMind（Polygonal Mind / medieval-fair、CC0）
- BLAKE3: `fa24848fa7b971718ce6e8dc9bd2d211808fd64e4ddf555feb66d4f82ee1e72a`
- 詳細: [`client/fixtures/README.md`](../../../client/fixtures/README.md)
- Icons: [`client/app/assets/icons/README.md`](../../../client/app/assets/icons/README.md)

環境変数: `ASSETS_URL`（既定 `:4004`）、任意 `ZENOH_CONNECT`（既定 `tcp/127.0.0.1:7447`）、任意 `AUTH_USER`（`alice`→entity 3 / `bob`→4）、任意 `ALCHEMY_BLOB_BLAKE3`、任意 `ALCHEMY_HOST_PID`、任意 `ALCHEMY_FORCE_DESKTOP=1`、任意 `ALCHEMY_OPENXR_LOADER` / `STEAM_PATH`。

## 関連

- ワイヤ契約: [protocol.md](protocol.md)
- 転送: [zenoh-router.md](zenoh-router.md)
- Room Token: [world-server.md](world-server.md)
- Blob CAS: [assets-server.md](assets-server.md)
- Slice: [x0014](../roadmap/slices/x0014-lan-two-client.md) · [x0013](../roadmap/slices/x0013-desktop-grab.md) · [x0011](../roadmap/slices/x0011-grab-hold.md) · [x0010](../roadmap/slices/x0010-grabbable-boxes.md)
