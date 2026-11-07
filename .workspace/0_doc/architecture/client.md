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
| `client/host-server` | オンデマンド子プロセスの計算ホスト（Rapier + user_root loco） |
| `client/app/src/host_supervisor.rs` | 親 client からの spawn / ready / IPC shutdown / Job Object |
| `client/fixtures/` | Coin_PolygonalMind GLB（CC0） |
| `../protocol` | 共有ワイヤ契約（workspace 外） |

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| Zenoh: Manifest / Presence / frame Subscribe、snapshot get、input Publish（`ROOM_ID`・x0017 / x0023） | リレー機能（他人の通信の中継は禁止）・テナント名前空間 |
| MainHub / Leave は **空シェル**（床・世界 mesh なし；x007d）。Join/Host 後の受理 Manifest mesh blake3 → assets GET（Bearer/PlaceRead・x001d/x0048）→ `try_apply_world_remount`（`remount_generation` + `wire_cap` + `remounted_blake3` / status と queue を同ロック）→ `SceneGpu::replace_world_mesh`。GET 失敗は placeholder + sticky status（同一 blake3 のセッション内再試行なし・Leave / re-Join で回復） | Manifest 全 mesh のマルチ GLB；LOD / KTX2 / マテリアル本格 / スキン；細切れ転送の壁時計 30s 上限；mid-GET HTTP abort |
| Frame: `host_sig` 検証・finite・tick 規則。Manifest は **`world_seal`** + **HostAttestation** + **`(generation, revision)` 単調ゲート**後にのみ `host_pub` 採用；検証失敗は **discard**（入室維持・x0038 / x003a）。world JWKS は soft SWR + **hard stale 300s** + unknown-kid 冷却（受信パス HTTP なし・x0044） | 予測補間 / jitter buffer・欠落デルタ再送 |
| Presence: pin + `host_sig` + Manifest generation 一致 / revision 単調（x0025 / x003a）。失敗は適用しない（discard） | Presence の world 署名；Presence 単独での generation 上昇 |
| Zenoh sample: class 別 **decode byte/count 上限**超過は drop（x003b） | router 側 max payload |
| Manifest entries → mesh blake3 / box=`half_extent` 描画 / Grab=`SCENE_FLAG_GRABBABLE`（x0027）。分類は `alchemy-shared-sim` `ManifestIndex` / `EntityKind`（x0030）。entity 表 upsert/remove。late join / **tick gap** は snapshot 置換（クールダウン 500ms・x0024） | loco の client 解釈・boot 後の mesh blake3 再取得 |
| Desktop: WASD/Space/C → Intent（look yaw 相対）。クリックで Confined（失敗時 Locked）→ マウスルック。look 中 RMB → Grab（x0013） | 感度 UI / クロスヘア必須 |
| Presence で自 entity 解決→カメラ；他 root は色付きボックス（x0015）。欠落で unset（x0016）。Home タブ login/register/logout（x0018）。logout は Leave + liveliness undeclare（x0019）。`AUTH_USER` / `ALCHEMY_AUTH_TOKEN` はサイレント **login のみ**（部屋は MainHub → Join a host / Host a room）。`ZENOH_CONNECT` LAN | 5人以上・アバター GLB・XR 文字入力 |
| OpenXR: 左右 thumbstick（aim 相対・倒し量・加算）→ Intent | Vive トラックパッド専用 / ホバー |
| OpenXR: `XR_EXT_render_model` 時に実機コントローラ GLB（ルート pose + 埋め込み baseColor + animatable node） | SteamVR パス直読み／PBR（MR/normal 等）／手トラ |
| OpenXR: ダッシュ開→UI レーザー；閉→ワールドレーザー＋ Grab hold（squeeze／Select）（x0011） | Steal／回転追従／Facet 並べ替え／WorldRoot／CompositionLayerQuad |
| auth login → Room Token は **bind Intent のみ**（Token 発行時に `session_pub`→`cnf.spk`・x003e）。以降は Ed25519 session（x0023）。host generation 交代は新鍵を pending に置き、Token 到着後に鍵+Token 一括 swap（取得中は旧鍵+旧 Token） | OSC / Dual Editor；debug `ALCHEMY_ROOM_TOKEN` bypass は **cnf 無しだと bind 失敗**（本物 Token を使う） |
| `HostSupervisor`: Join では host 未起動。Start / Worlds Host は `ensure_running_for_room`（`ROOM_ID` 不一致なら再起動）。ready は IPC bind 直後。Windows Job Object / Linux PDEATHSIG（`ALCHEMY_HOST_BIN`）。Exit は Handover stub → IPC SHUTDOWN。Leave/Logout は host 維持。Exit Client only 廃止 | deep link / トレイ / 本実装 Handover / host crash 自動再起動 / `cargo run` 道連れ / launcher discovery；入室維持のままの Host Rescue |
| host: world `GET …/scene` → Rapier `from_entries` + sealed Manifest 公開（x0027）。60Hz + session gate + `is_finite` + Grab 開始距離（x0021）+ **hold aim/offset クランプ**（x0041）+ **Intent rate**（x0040；bind は per-ZenohId source・map cap・idle=成功消費基準・x0077；host 全体 JWT soft-global 無し）+ Presence + Leave / jti liveliness / TTL（x0022）+ signed 30Hz 純デルタ + snapshot。Room Token JWKS は soft SWR + **hard stale 300s**（x0040）。attestation / seal は **credential worker**（x003d）；Frame/Manifest/Presence **`put` は `zenoh-publish`**（x0042・物理は pose ジョブ投入のみ・encode は worker 内 protobuf）。物理は gate + `hard_expire_at` を毎 tick 評価。一時 HTTP 失敗ではプロセスを殺さず、hard expiry 後は配信停止。再開時は `last_sent` から catch-up。`ALCHEMY_AUTH_TOKEN` は長命前提。Grab on/off は `alchemy-shared-sim` **SlotGraph** と同期（x0056/x0057）；`pass_held` / `place_held`（x005a）；一般 `reparent` + 循環拒否（x0059）；`SlotUndoStack` API（x005b・UI 入口は後続）。**x006d**: Zenoh `…/inventory/spawn` は Room AEAD（`InventorySpawn`）；spawn worker が host role 検証後に world scene 追記（物理ループ外）；physics は scene+reseal 先→Rapier（拒否時も reseal）。**x007b**: `…/inventory/despawn`（`InventoryDespawn`）→ world DELETE → scene+reseal → Rapier remove | Formula / 予測補間 / verifier / 本格 Walk / Steal / floor Manifest 化 / rkyv·bytemuck ワイヤ / 短命静的 auth JWT の自動更新 / Intent strike・一時 ban / 未 bind steady の Ed25519 洪水 / bind source map 満杯かつ成功消費で全行 hot のときの新規 source deny・JWT CPU / Prefab 多子 take-out / Frame parent フィールド |
| metrics（x0049）: `ALCHEMY_METRICS_ADDR=127.0.0.1:…` 時のみ専用 localhost `GET /metrics`（tick lateness / late>5ms counter / frame seen+age / JWKS age·hard-stale）。未設定は起動しない | OTLP / 公網スクレイプ / 高 cardinality label（`sub`/`room_id`） |
| ESC（Desktop）/ Menu（OpenXR）ダッシュ・下タブ Home/Inventory/Settings/Exit・Pinned Exit・Home アカウント殻（x0018）・**Inventory**（Drive take-out・x006d + scene mesh put-back・x006e Desktop；put-back 後も entity は部屋に残る；**Despawn**・x007b で部屋から削除；Take out は **blob のみ**で prefab 往復 UI は未）・Button hover・**単一 Exit**（supervised host のみ Handover stub + IPC；外部/CI host は触らない）。Esc でダッシュ開時は look／Grab 解放 | Facet 並べ替え / Context / Primary ツール本体 / XR Inventory / Prefab Take out / XR 仮想キーボード / 認証付きリモート shutdown |
| Home ログイン後: MainHub で **Join a host** / **Host a room**（x007c）。作成パネルでセッション名 + **アクセスレベル**（=`visibility`）→ Host で create/PUT + 自 host（x0037）。owner 同一アカウント時のみ選択レベルが載る；非 owner は既存 ACL のまま入室し status で区別。ログイン（UI / silent / remember-me）は自動入室しない | UI アート最終化 / テンプレ本実装 / XR Start HTTP / 表示名の永続 |
| Worlds タブ（x007c）: カタログ＝見える／Join＝入れる／Live＝読める／Host＝回す。Join as guest と Host 分離（Rescue は owner・実入室のみ）。Home は Open Worlds／Create 誘導；固定 ROOM_ID Join/Start は Settings Advanced | XR Worlds HTTP；Host Rescue の入室維持パス；Home からの Join 主導線 |
| Room key epoch / KeyStale（x007e）: Token の `room_key_epoch` を AEAD AAD に束縛；refresh で epoch 前進（**`0→1` 含む**）→ KeyStale（**live AEAD は旧鍵維持**・新鍵は Rejoin で commit）+ remount_generation bump。KeyStale overlay / `worlds_key_stale` 同期は **Desktop** | KeyStale 中の透過的無停止鍵切替；XR 上の KeyStale overlay / Rejoin HTTP（XR ゲストは FRAME_STALL 等に依存） |
| visibility 正本は world DB。host `ROOM_VISIBILITY` は **初回 create 既定**；owner の client PUT が後勝ち。Start session は visibility Applied → local host spawn → join（LeftExisting は spawn しない） | 異アカウントが先に create したときの host 昇格 |
| OpenXR ランタイムあり → XR session **+ Desktop companion**（[x0053](../roadmap/slices/x0053-xr-desktop-hotswitch.md)）。なし／`ALCHEMY_FORCE_DESKTOP` → Desktop のみ。**READY 前は Desktop が Intent 所有**（SteamVR 待ちのデッドゾーンなし）。READY 後 F8 / Settings / presence で切替。`ALCHEMY_HOTSWITCH=0` で auto 初期 OFF。**init 失敗は Desktop Solo 相当継続**。Zenoh 正本は Desktop のみ。XR スレッド panic でも `xr_session_live` をクリア | WSL2 実 HMD；Quest/WebXR 同等 UX；Virtual Desktop presence 完全一致；壊れた近接センサの万能救済（auto 抑止＋F8）；companion 窓の VR 中ミラー描画コスト；Zenoh 無し時の Leave/host shutdown put |
| OpenXR: XR Vulkan を wgpu-hal で採用し SceneGpu dual-eye＋UI／レーザー／コントローラ overlay（x0052 既定）。ash は enable／interop のみ。Desktop 非 active 時は empty layers で session 維持 | multiview；CompositionLayerQuad；Bevy；単一 GPU での窓+swapchain 共有 |

## チャネル分離（C-C + E + x0009）

| キー | 頻度 | 用途 |
|:---|:---|:---|
| `alchemy/dev/room/{ROOM_ID}/manifest` | 低（host は 5s 再送で late joiner 補助） | `entity_id → blake3` + `locomotion` |
| `alchemy/dev/room/{ROOM_ID}/presence` | 低（割当・解放変更 + 5s 再送） | `sub → user_root entity`（x0015 / 解放 x0016+x0019） |
| `alchemy/dev/room/{ROOM_ID}/alive/{sub}/{jti}` | liveliness | client token；host は検証済み jti 一致時のみ reclaim（x0022） |
| `alchemy/dev/room/{ROOM_ID}/frame` | 高（動いたものだけ）・**latest デルタ** | 署名付き pose upserts / removes。tick 欠落時は適用せず snapshot（x0023 / x0024） |
| `alchemy/dev/room/{ROOM_ID}/snapshot` | queryable | late join / 再同期用フル pose（`RenderFrame`・x0023 / x0024） |
| `alchemy/dev/room/{ROOM_ID}/input` | 入力 | session 署名 Intent（Token は bind / refresh 時のみ・x0023） |
| `alchemy/dev/room/{ROOM_ID}/host_control` | 低（既定オフ） | host 側 opt-in 購読のみ（x001e）。client Exit は `HostSupervisor` localhost IPC（supervised のみ） |

## Desktop バインド（x0009）

| 入力 | 役割 |
|:---|:---|
| Esc | ダッシュ開閉（開いている間は移動 Intent = 0） |
| F8 | XR companion 時: VR↔Desktop present 切替（x0053。Resonite 相当） |
| WASD | 前/左/後/右（固定 look・yaw=0 相対）※Desktop が入力所有のときのみ |
| Space / C | 上昇 / 下降（Noclip・Fly） |
| Y / T / R | Primary / Context / Secondary（キーのみスタブ） |
| マウス | ダッシュ開: UI。閉: LMB で look capture、移動で yaw/pitch（x0012）；look 中は箱を下向き照準（明るい）＋ LMB/RMB/E 長押しで Grab（x0013）。VR present 中は look/Grab を Desktop 側で出さない |

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

環境変数: `ASSETS_URL`（既定 `:4004`）、任意 `ROOM_ID`（既定 `local`・Zenoh キーと Room Token に反映・x0017）、任意 `ZENOH_CONNECT`（既定 `tcp/127.0.0.1:7447`）、任意 `AUTH_USER` / `AUTH_PASSWORD` / `ALCHEMY_AUTH_TOKEN`（設定時は Home UI をスキップしてサイレント **login のみ**・部屋 Join は MainHub・x0018/x007c。**release では seed 既定なし**・x001e）、任意 `ALCHEMY_HOST_BIN`（HostSupervisor 子の実バイナリ。未設定時は隣接 `host-server` または `cargo run`）、任意 `ALCHEMY_FORCE_DESKTOP=1`、任意 `ALCHEMY_HOTSWITCH=0`（x0053 auto 初期 OFF・F8 は有効）、任意 `ALCHEMY_OPENXR_LOADER` / `STEAM_PATH`。host 側のみ任意 `ALCHEMY_HOST_CONTROL=1`（Zenoh shutdown opt-in）・`ALCHEMY_BLOB_BLAKE3`（canonical ManifestSeal と非互換・拒否用途）。  
未設定時は Desktop で Step1/Step2 → login/register → **MainHub**（Join a host / Host a room）。自動入室しない。

## 関連

- ワイヤ契約: [protocol.md](protocol.md)
- 転送: [relay-server.md](relay-server.md)
- Room Token: [world-server.md](world-server.md)
- Blob CAS: [assets-server.md](assets-server.md)
- Slice: [x0016](../roadmap/slices/x0016-presence-slot-reclaim.md) · [x0015](../roadmap/slices/x0015-presence-slot-assign.md) · [x0014](../roadmap/slices/x0014-lan-two-client.md) · [x0013](../roadmap/slices/x0013-desktop-grab.md)
