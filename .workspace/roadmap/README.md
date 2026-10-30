# Roadmap

縦スライス単位の進捗目次。各 Slice の詳細（保証・Phase・起動）は個別ファイルへ。

## ID 規約（`x` + 16進）

1000 件超を見据え、Slice の正本 ID は次で固定する。

| 項目 | 形式 | 例 |
|:---|:---|:---|
| **ID** | `x` + **4桁ゼロ埋め・小文字** 16進 | `x0001` … `x03e8`（=1000₁₀）… `xffff` |
| **slug** | kebab-case | `system-ui-dash` |
| **ファイル** | `slices/xNNNN-slug.md` | [`slices/x0007-system-ui-dash.md`](slices/x0007-system-ui-dash.md) |
| **ブランチ** | `feat/xNNNN-slug` | `feat/x0007-system-ui-dash` |
| **見出し** | `# xNNNN — …` | `# x0007 — system_ui ダッシュ` |

- ID は **発行後に変えない**（並べ替え・参照の安定性）
- 次の空き番号は目次の最大 ID + 1（いまは次 = **`x0050`**）

- 旧 A〜E 表記は互換のための別名。新規文書・ブランチ・コミットでは **`xNNNN` を使う**

### タグ（front matter / ファイル先頭メタ）

| キー | 値 | 説明 |
|:---|:---|:---|
| `id` | `xNNNN` | 必須 |
| `slug` | kebab | 必須 |
| `status` | `planned` / `active` / `done` / `deferred` | 必須 |
| `area` | 複数可 | `auth` `world` `assets` `protocol` `zenoh` `host` `client` `system_ui` `sim` `relay` `launcher` `docs` `ci` |
| `surface` | 任意・複数可 | `desktop` `vr` など |
| `depends_on` | `xNNNN` のリスト | 任意 |

例:

```yaml
id: x0007
slug: system-ui-dash
status: done
area: [client, system_ui]
surface: [desktop]
depends_on: [x0006]
```

### 目次の持ち方

- この README は **直近・現行** を載せる（全件巨大表にしない）
- 詳細はすべて [`slices/`](slices/) 配下
- 件数が増えたら area 別インデックスを追加してよい

## 目次

| ID | 旧称 | 状態 | 要約 | 詳細 |
|:---|:---|:---|:---|:---|
| x0001 | A | done | ローカル Zenoh 縦スライス（Publish → Subscribe → 描画） | [x0001-zenoh-local-frame.md](slices/x0001-zenoh-local-frame.md) |
| x0002 | B | done | Room Token 門番（auth JWT 必須） | [x0002-room-token-gate.md](slices/x0002-room-token-gate.md) |
| x0003 | C-A | done | assets-server 最小（CAS + Quota） | [x0003-assets-cas-quota.md](slices/x0003-assets-cas-quota.md) |
| x0004 | C-B | done | Coin GLB を wgpu 表示 | [x0004-coin-glb-wgpu.md](slices/x0004-coin-glb-wgpu.md) |
| x0005 | C-C | done | Manifest（低頻度）＋ RenderFrame デルタ（高頻度） | [x0005-manifest-delta.md](slices/x0005-manifest-delta.md) |
| x0006 | D | done | Rapier 最小＋ `is_finite`（内部 60Hz / 配信 30Hz） | [x0006-rapier-finite-tick.md](slices/x0006-rapier-finite-tick.md) |
| x0007 | E | done | system_ui ダッシュ（自前 Canvas / ESC） | [x0007-system-ui-dash.md](slices/x0007-system-ui-dash.md) |
| x0008 | — | done | OpenXR 最小（ダッシュ＋Exit / SteamVR） | [x0008-openxr-dash-exit.md](slices/x0008-openxr-dash-exit.md) |
| x0009 | — | done | Desktop Noclip（Manifest loco + WASD） | [x0009-desktop-noclip-loco.md](slices/x0009-desktop-noclip-loco.md) |
| x000a | — | done | VR 親指スティック Noclip（aim 相対） | [x000a-vr-thumbstick-noclip.md](slices/x000a-vr-thumbstick-noclip.md) |
| x000b | — | done | OpenXR Render Model でコントローラ表示 | [x000b-vr-controller-render-models.md](slices/x000b-vr-controller-render-models.md) |
| x000c | — | done | コントローラ GLB baseColor テクスチャ | [x000c-vr-controller-glb-textures.md](slices/x000c-vr-controller-glb-textures.md) |
| x000d | — | done | コントローラ animatable node（トリガー／スティック） | [x000d-vr-controller-animatable-nodes.md](slices/x000d-vr-controller-animatable-nodes.md) |
| x000e | — | done | VR UI レーザーポインタ（簡易・ダッシュ開中） | [x000e-vr-ui-laser-pointer.md](slices/x000e-vr-ui-laser-pointer.md) |
| x000f | — | done | Dash Facet 最小（個人・Pinned Exit・Button hover） | [x000f-dash-facet-minimal.md](slices/x000f-dash-facet-minimal.md) |
| x0010 | — | done | Grabbable ボックス最小（3×3・コライダー・レイヒット） | [x0010-grabbable-boxes.md](slices/x0010-grabbable-boxes.md) |
| x0011 | — | done | Grab hold（XR kinematic 追従・放下） | [x0011-grab-hold.md](slices/x0011-grab-hold.md) |
| x0012 | — | done | Desktop マウスルック（Pointer Lock） | [x0012-desktop-mouse-look.md](slices/x0012-desktop-mouse-look.md) |
| x0013 | — | done | Desktop Grab（RMB・look 前方レイ） | [x0013-desktop-grab.md](slices/x0013-desktop-grab.md) |
| x0014 | — | done | LAN 2-client presence（sub 別 user_root） | [x0014-lan-two-client.md](slices/x0014-lan-two-client.md) |
| x0015 | — | done | Presence 動的スロット割当（最大4・固定マップ廃止） | [x0015-presence-slot-assign.md](slices/x0015-presence-slot-assign.md) |
| x0016 | — | done | Presence スロット解放（Intent 無音 TTL 3s） | [x0016-presence-slot-reclaim.md](slices/x0016-presence-slot-reclaim.md) |
| x0017 | — | done | ROOM_ID → Zenoh キー本配線 | [x0017-room-id-zenoh-keys.md](slices/x0017-room-id-zenoh-keys.md) |
| x0018 | — | done | client アカウント殻（login / logout / 最小 register） | [x0018-client-auth-session.md](slices/x0018-client-auth-session.md) |
| x0019 | — | done | 明示 Leave / Zenoh liveliness | [x0019-explicit-leave-liveliness.md](slices/x0019-explicit-leave-liveliness.md) |
| x001a | — | done | HTTPS 入口（Cloudflare Tunnel） | [x001a-https-edge-tunnel.md](slices/x001a-https-edge-tunnel.md) |
| x001b | — | done | Zenoh WAN（ConoHa zenohd + IP allowlist） | [x001b-zenoh-wan-conoha.md](slices/x001b-zenoh-wan-conoha.md) |
| x001c | — | done | assets CAS → R2（S3 互換バックエンド） | [x001c-assets-cas-r2.md](slices/x001c-assets-cas-r2.md) |
| x001d | — | done | R2 署名 URL 直結（GET 302） | [x001d-assets-r2-presign.md](slices/x001d-assets-r2-presign.md) |
| x001e | — | done | P0 信頼境界（Tunnel prod / host_control / seed / assets CI） | [x001e-p0-trust-boundary.md](slices/x001e-p0-trust-boundary.md) |
| x001f | — | done | Hermetic Compose 縦 E2E（login→token→input→frame） | [x001f-hermetic-compose-e2e.md](slices/x001f-hermetic-compose-e2e.md) |
| x0020 | — | done | Desktop client 手通し（日常スタック正本 + smoke） | [x0020-desktop-handthrough.md](slices/x0020-desktop-handthrough.md) |
| x0021 | — | done | P1 デモゲート（Token refresh / JWKS bg / clamp / host role） | [x0021-p1-demo-gate.md](slices/x0021-p1-demo-gate.md) |
| x0022 | — | done | liveliness セッション紐付け + WAN TLS（P2 前半） | [x0022-liveliness-wan-tls.md](slices/x0022-liveliness-wan-tls.md) |
| x0023 | — | done | P2 後半（session binding / Frame 検証 / snapshot） | [x0023-p2-session-frame.md](slices/x0023-p2-session-frame.md) |
| x0024 | — | done | 配送クラス契約（peer-delivery + frame 欠落→snapshot） | [x0024-delivery-class.md](slices/x0024-delivery-class.md) |
| x0025 | — | done | host_pub 証明（world attestation + Presence host_sig） | [x0025-host-pub-attestation.md](slices/x0025-host-pub-attestation.md) |
| x0026 | — | done | Manifest 全体の world seal（本体改ざん検知・P4 なし） | [x0026-manifest-world-seal.md](slices/x0026-manifest-world-seal.md) |
| x0027 | — | done | SceneManifest 駆動（P4 最小 / D2） | [x0027-manifest-driven.md](slices/x0027-manifest-driven.md) |
| x0028 | — | done | Room 永続（Ash + SQLite / D4） | [x0028-room-persistence.md](slices/x0028-room-persistence.md) |
| x0029 | — | done | RenderFrame 回転 + スケール（E1） | [x0029-renderframe-trs.md](slices/x0029-renderframe-trs.md) |
| x002a | — | done | role → sim capability（D4 残り） | [x002a-role-sim-capability.md](slices/x002a-role-sim-capability.md) |
| x002b | — | done | Grab OBB（描画 TRS 整合） | [x002b-grab-obb.md](slices/x002b-grab-obb.md) |
| x002c | — | done | Manifest 初期 quat + scale | [x002c-manifest-initial-trs.md](slices/x002c-manifest-initial-trs.md) |
| x002d | — | done | P3 衛生（Quota / Refresh / Parsers / audit / `-o` / protoc） | [x002d-p3-hygiene.md](slices/x002d-p3-hygiene.md) |
| x002e | — | done | P3 境界・契約 + ランタイム（ClientIp / room_id / poison / UI worker） | [x002e-p3-boundary-runtime.md](slices/x002e-p3-boundary-runtime.md) |
| x002f | — | done | P3 alchemy_auth_client JWKS 共通化 + undocumented_unsafe deny | [x002f-p3-auth-client-unsafe.md](slices/x002f-p3-auth-client-unsafe.md) |
| x0030 | — | done | shared-sim / EntityKind（P4-2） | [x0030-shared-sim-entity-kind.md](slices/x0030-shared-sim-entity-kind.md) |
| x0031 | — | done | trust-boundary threat model（P4-3） | [x0031-trust-boundary-threat-model.md](slices/x0031-trust-boundary-threat-model.md) |
| x0032 | — | done | launcher 最小実装（P4-4） | [x0032-launcher-minimal.md](slices/x0032-launcher-minimal.md) |
| x0033 | — | done | Follow / Friendship（auth SSoT・相互→申請） | [x0033-friendship-follow.md](slices/x0033-friendship-follow.md) |
| x0034 | — | done | Room visibility + Token 門番（private/friends/public + S2S） | [x0034-visibility-token-gate.md](slices/x0034-visibility-token-gate.md) |
| x0035 | — | done | friends_plus / followers / followers_plus（2hop） | [x0035-visibility-hops.md](slices/x0035-visibility-hops.md) |
| x0036 | — | done | lan 公開範囲（外部・Tunnel 拒否） | [x0036-lan-visibility.md](slices/x0036-lan-visibility.md) |
| x0037 | — | done | client アクセスレベル UI（新規ワールド作成・仮置き） | [x0037-client-access-level-ui.md](slices/x0037-client-access-level-ui.md) |
| x0038 | — | done | 検証失敗は破棄（leave→discard）+ 否定系 E2E | [x0038-leave-to-discard.md](slices/x0038-leave-to-discard.md) |
| x0039 | — | done | canonical ManifestSeal（署名 oracle 廃止） | [x0039-canonical-manifest-seal.md](slices/x0039-canonical-manifest-seal.md) |
| x003a | — | done | generation / revision 単調ゲート（replay 拒否） | [x003a-generation-revision-gate.md](slices/x003a-generation-revision-gate.md) |
| x003b | — | done | decode limit + prod S2S + world テスト DB 隔離 | [x003b-decode-limits-ops-hygiene.md](slices/x003b-decode-limits-ops-hygiene.md) |
| x003c | — | done | ZIP/launcher: Coin mesh は assets DL 必須（ensure + Manifest join republish） | [x003c-assets-required-mesh.md](slices/x003c-assets-required-mesh.md) |
| x003d | — | done | host credential worker 分離（物理ループから HTTP 除去・hard expiry で配信停止） | [x003d-credential-worker.md](slices/x003d-credential-worker.md) |
| x003e | — | done | Room Token `cnf.spk`（session_pub 束縛・P1-4） | [x003e-room-token-cnf.md](slices/x003e-room-token-cnf.md) |
| x003f | — | done | CAS object/ref 分離 + reserved_bytes put 補償（P1-2） | [x003f-cas-object-ref-reserve.md](slices/x003f-cas-object-ref-reserve.md) |
| x0040 | — | done | Intent rate limit + JWKS hard stale（P1-5） | [x0040-intent-rate-jwks-hard-stale.md](slices/x0040-intent-rate-jwks-hard-stale.md) |
| x0041 | — | done | Grab 保持中 aim / offset 距離クランプ（P1-6） | [x0041-grab-hold-aim-clamp.md](slices/x0041-grab-hold-aim-clamp.md) |
| x0042 | — | done | Zenoh network worker（物理から put 分離・P1-3 残り） | [x0042-zenoh-network-worker.md](slices/x0042-zenoh-network-worker.md) |
| x0043 | — | done | `/internal/*` ingress 404 + Social rate limit（P2-2） | [x0043-internal-ingress-social-rl.md](slices/x0043-internal-ingress-social-rl.md) |
| x0044 | — | done | client world_jwks SWR / hard / unknown-kid（P2-3） | [x0044-client-world-jwks-cooldown.md](slices/x0044-client-world-jwks-cooldown.md) |
| x0045 | — | done | host Room Token 否定系テスト（P3-8） | [x0045-room-token-negative-tests.md](slices/x0045-room-token-negative-tests.md) |
| x0046 | — | done | adversarial E2E（偽 Manifest / stale-rev / oversize・P3-1） | [x0046-adversarial-e2e.md](slices/x0046-adversarial-e2e.md) |
| x0047 | — | done | launcher CI + 再起動ポリシー単体テスト（P3-2） | [x0047-launcher-ci-restart-policy.md](slices/x0047-launcher-ci-restart-policy.md) |
| x0048 | — | done | asset error placeholder + HTTP timeout（P3-3） | [x0048-asset-placeholder-http-timeout.md](slices/x0048-asset-placeholder-http-timeout.md) |
| x0049 | — | done | metrics 最小（localhost Prometheus text・P3-4） | [x0049-minimal-metrics.md](slices/x0049-minimal-metrics.md) |
| x004a | — | done | README / roadmap 鮮度（P3-5・vision 本文は未） | [x004a-docs-freshness.md](slices/x004a-docs-freshness.md) |
| x004b | — | done | ClientIp 共有化 + HTTP inspect 除去（P3-6） | [x004b-client-ip-shared-inspect.md](slices/x004b-client-ip-shared-inspect.md) |
| x004c | — | done | cargo-fuzz protobuf / JWT（P3-9） | [x004c-cargo-fuzz-protobuf-jwt.md](slices/x004c-cargo-fuzz-protobuf-jwt.md) |
| x004d | — | done | USER_ROOT_SLOTS → Manifest（P3-10・帯は protocol） | [x004d-user-root-slots-manifest.md](slices/x004d-user-root-slots-manifest.md) |
| x004e | — | done | Relay Agent MVP（P2-1: mTLS + Allocation + wire_cap） | [x004e-relay-agent-mvp.md](slices/x004e-relay-agent-mvp.md) |
| x004f | — | done | BlobGrant MVP（P4-5 案 A: 台帳 + grant_required。目標は案 B） | [x004f-blob-grant-mvp.md](slices/x004f-blob-grant-mvp.md) |
| x0050 | — | done | XR swapchain → wgpu（P4-1 / 1/3・clear-only） | [x0050-xr-swapchain-wgpu.md](slices/x0050-xr-swapchain-wgpu.md) |
| x0051 | — | planned | XR SceneGpu 一本化（P4-1 / 2/3・dual-eye） | [x0051-xr-scene-gpu-unify.md](slices/x0051-xr-scene-gpu-unify.md) |
| x0052 | — | planned | XR vk_loop GPU 削除（P4-1 / 3/3） | [x0052-xr-vk-loop-gpu-remove.md](slices/x0052-xr-vk-loop-gpu-remove.md) |

**現在の次:** **[x0051](slices/x0051-xr-scene-gpu-unify.md)**（P4-1 描画統合 2/3）。並行候補: 案 B（GET 常時 Grant）/ launcher / verifier（保留）。  
**P3-7**（BLAKE3 ストリーミング・Room 作成上限）は **CI が完了しなくなるためマージしない**（[improvement-plan](../0_doc/evaluation/improvement-plan-2026-10-28.md)）。

次の空き番号は **`x0053`**。


## 関連

| 文書 | 内容 |
|:---|:---|
| [ローカル CI](ci.md) | `make ci` など |
| [architecture/overview.md](../0_doc/architecture/overview.md) | 目標アーキテクチャ |
| [README.md](../../README.md) | リポジトリ入口・起動手順 |

## 管理ルール

- **1 Slice = 1 ファイル**（`slices/xNNNN-slug.md`。保証する／しない・Phase・起動をそのファイルに閉じる）
- 状態が変わったら **この目次の表** と当該 Slice ファイルの両方を更新する
- 新 Slice は **`xNNNN` を採番**し `slices/xNNNN-slug.md` を追加、目次行を足す
- ブランチ名は `feat/xNNNN-slug`
- アーキテクチャ正本へのリンクは `../../0_doc/architecture/`（slices 内から）を使う
