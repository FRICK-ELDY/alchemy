# client

> **正本**: [`client/`](../../../client/)（Cargo workspace）  
> **ステータス**: x0027（Manifest 駆動）+ x0026 seal + x0025 attestation + x0024 gap→snapshot  
 
> **配送契約**: [`protocol/peer-delivery.md`](../../../protocol/peer-delivery.md)

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
| Zenoh: Manifest / Presence / frame Subscribe、snapshot get、input Publish（`ROOM_ID`・x0017 / x0023） | ルーター機能（他人の通信の中継は禁止）・テナント名前空間 |
| Manifest の blake3 → assets GET（Bearer + CAS；s3 時は 302 追従・x001d；**connect 5s / read 30s idle timeout**・x0048）→ BLAKE3 → wgpu。認証失敗時は **error placeholder**（unit cube）+ sticky Home status。Coin fixture は未認証 Home、または **debug + `ALCHEMY_ALLOW_FIXTURE_FALLBACK`**（download / blake3 / decode）。**GET は認証済み読み取り capability**（owner 照合なし・[assets-server.md](assets-server.md)；所有者 grant は [assets-server-draft.md](assets-server-draft.md)） | LOD / KTX2 / マテリアル本格 / スキン；live mesh swap / 再認証後の自動再取得；細切れ転送の壁時計 30s 上限 |
| Frame: `host_sig` 検証・finite・tick 規則。Manifest は **`world_seal`** + **HostAttestation** + **`(generation, revision)` 単調ゲート**後にのみ `host_pub` 採用；検証失敗は **discard**（入室維持・x0038 / x003a）。world JWKS は soft SWR + **hard stale 300s** + unknown-kid 冷却（受信パス HTTP なし・x0044） | 予測補間 / jitter buffer・欠落デルタ再送 |
| Presence: pin + `host_sig` + Manifest generation 一致 / revision 単調（x0025 / x003a）。失敗は適用しない（discard） | Presence の world 署名；Presence 単独での generation 上昇 |
| Zenoh sample: class 別 **decode byte/count 上限**超過は drop（x003b） | router 側 max payload |
| Manifest entries → mesh blake3 / box=`half_extent` 描画 / Grab=`SCENE_FLAG_GRABBABLE`（x0027）。分類は `alchemy-shared-sim` `ManifestIndex` / `EntityKind`（x0030）。entity 表 upsert/remove。late join / **tick gap** は snapshot 置換（クールダウン 500ms・x0024） | loco の client 解釈・boot 後の mesh blake3 再取得 |
| Desktop: WASD/Space/C → Intent（look yaw 相対）。クリックで Confined（失敗時 Locked）→ マウスルック。look 中 RMB → Grab（x0013） | 感度 UI / クロスヘア必須 |
| Presence で自 entity 解決→カメラ；他 root は色付きボックス（x0015）。欠落で unset（x0016）。Home タブ login/register/logout（x0018）。logout は Leave + liveliness undeclare（x0019）。`AUTH_USER` / `ALCHEMY_AUTH_TOKEN` は CI サイレント login。`ZENOH_CONNECT` LAN | 5人以上・アバター GLB・XR 文字入力 |
| OpenXR: 左右 thumbstick（aim 相対・倒し量・加算）→ Intent | Vive トラックパッド専用 / ホバー |
| OpenXR: `XR_EXT_render_model` 時に実機コントローラ GLB（ルート pose + 埋め込み baseColor + animatable node） | SteamVR パス直読み／PBR（MR/normal 等）／手トラ |
| OpenXR: ダッシュ開→UI レーザー；閉→ワールドレーザー＋ Grab hold（squeeze／Select）（x0011） | Steal／回転追従／Facet 並べ替え／WorldRoot／CompositionLayerQuad |
| auth login → Room Token は **bind Intent のみ**（Token 発行時に `session_pub`→`cnf.spk`・x003e）。以降は Ed25519 session（x0023）。host generation 交代は新鍵を pending に置き、Token 到着後に鍵+Token 一括 swap（取得中は旧鍵+旧 Token） | OSC / Dual Editor；debug `ALCHEMY_ROOM_TOKEN` bypass は **cnf 無しだと bind 失敗**（本物 Token を使う） |
| launcher（x0032）: host/client 分離 spawn・IPC shutdown。Exit Host は `ALCHEMY_HOST_IPC_*` 優先 | deep link / トレイ / handoff |
| host: world `GET …/scene` → Rapier `from_entries` + sealed Manifest 公開（x0027）。60Hz + session gate + `is_finite` + Grab 開始距離（x0021）+ **hold aim/offset クランプ**（x0041）+ **Intent rate**（x0040）+ Presence + Leave / jti liveliness / TTL（x0022）+ signed 30Hz 純デルタ + snapshot。Room Token JWKS は soft SWR + **hard stale 300s**（x0040）。attestation / seal は **credential worker**（x003d）；Frame/Manifest/Presence **`put` は `zenoh-publish`**（x0042・物理は pose ジョブ投入のみ・encode は worker 内 protobuf）。物理は gate + `hard_expire_at` を毎 tick 評価。一時 HTTP 失敗ではプロセスを殺さず、hard expiry 後は配信停止。再開時は `last_sent` から catch-up。`ALCHEMY_AUTH_TOKEN` は長命前提 | Formula / 予測補間 / verifier / 本格 Walk / Steal / floor Manifest 化 / rkyv·bytemuck ワイヤ / 短命静的 auth JWT の自動更新 / Intent strike・一時 ban / 未 bind steady の Ed25519 洪水 |
| metrics（x0049）: `ALCHEMY_METRICS_ADDR=127.0.0.1:…` 時のみ専用 localhost `GET /metrics`（tick lateness / late>5ms counter / frame seen+age / JWKS age·hard-stale）。未設定は起動しない | OTLP / 公網スクレイプ / 高 cardinality label（`sub`/`room_id`） |
| ESC（Desktop）/ Menu（OpenXR）ダッシュ・下タブ Home/Settings/Exit・Pinned Exit・Home アカウント殻（x0018）・Button hover・client 終了・host 終了は `ALCHEMY_HOST_PID`（Zenoh `host_control` は opt-in・x001e）。Esc でダッシュ開時は look／Grab 解放 | Facet 並べ替え / Context / Primary ツール本体 / XR 仮想キーボード / 認証付きリモート shutdown |
| Home ログイン後: 新規ワールド作成パネル（仮 UI）でセッション名 + **アクセスレベル**（=`visibility`）→ Start で create/PUT + guest 入室（x0037）。owner 同一アカウント時のみ選択レベルが載る；非 owner は既存 ACL のまま入室し status で区別。`ROOM_ID` 固定。ログインは自動入室しない（CI サイレントは従来どおり入室） | UI アート最終化 / テンプレ本実装 / XR Start HTTP / 表示名の永続 |
| visibility 正本は world DB。host `ROOM_VISIBILITY` は **初回 create 既定**；owner の client PUT が後勝ち。起動順は host 先（同一アカウント）を前提 | 異アカウントが先に create したときの host 昇格 |
| OpenXR ランタイムあり → XR、なし／`ALCHEMY_FORCE_DESKTOP` → Desktop | WSL2 実 HMD |

## チャネル分離（C-C + E + x0009）

| キー | 頻度 | 用途 |
|:---|:---|:---|
| `alchemy/dev/room/{ROOM_ID}/manifest` | 低（host は 5s 再送で late joiner 補助） | `entity_id → blake3` + `locomotion` |
| `alchemy/dev/room/{ROOM_ID}/presence` | 低（割当・解放変更 + 5s 再送） | `sub → user_root entity`（x0015 / 解放 x0016+x0019） |
| `alchemy/dev/room/{ROOM_ID}/alive/{sub}/{jti}` | liveliness | client token；host は検証済み jti 一致時のみ reclaim（x0022） |
| `alchemy/dev/room/{ROOM_ID}/frame` | 高（動いたものだけ）・**latest デルタ** | 署名付き pose upserts / removes。tick 欠落時は適用せず snapshot（x0023 / x0024） |
| `alchemy/dev/room/{ROOM_ID}/snapshot` | queryable | late join / 再同期用フル pose（`RenderFrame`・x0023 / x0024） |
| `alchemy/dev/room/{ROOM_ID}/input` | 入力 | session 署名 Intent（Token は bind / refresh 時のみ・x0023） |
| `alchemy/dev/room/{ROOM_ID}/host_control` | 低（既定オフ） | `ALCHEMY_HOST_CONTROL=1` のときのみ client→host `shutdown`（x001e。既定は `ALCHEMY_HOST_PID`） |

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

環境変数: `ASSETS_URL`（既定 `:4004`）、任意 `ROOM_ID`（既定 `local`・Zenoh キーと Room Token に反映・x0017）、任意 `ZENOH_CONNECT`（既定 `tcp/127.0.0.1:7447`）、任意 `AUTH_USER` / `AUTH_PASSWORD` / `ALCHEMY_AUTH_TOKEN`（設定時は Home UI をスキップしてサイレント login・x0018。**release では seed 既定なし**・x001e）、任意 `ALCHEMY_BLOB_BLAKE3`、任意 `ALCHEMY_HOST_PID`、任意 `ALCHEMY_HOST_CONTROL=1`（Zenoh shutdown・ローカル専用）、任意 `ALCHEMY_FORCE_DESKTOP=1`、任意 `ALCHEMY_OPENXR_LOADER` / `STEAM_PATH`。  
未設定時は Desktop で Home タブが開き、login/register 後に入室する。

## 関連

- ワイヤ契約: [protocol.md](protocol.md)
- 転送: [zenoh-router.md](zenoh-router.md)
- Room Token: [world-server.md](world-server.md)
- Blob CAS: [assets-server.md](assets-server.md)
- Slice: [x0016](../roadmap/slices/x0016-presence-slot-reclaim.md) · [x0015](../roadmap/slices/x0015-presence-slot-assign.md) · [x0014](../roadmap/slices/x0014-lan-two-client.md) · [x0013](../roadmap/slices/x0013-desktop-grab.md)
