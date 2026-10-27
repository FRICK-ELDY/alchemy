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
- 次の空き番号は目次の最大 ID + 1（いまは次 = **`x002e`**；x002d done）
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

**現在の次:** ClientIp / room_id / Mutex poison / UI HTTP worker / JWKS 共通化 / undocumented_unsafe・shared-sim / EntityKind。

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
