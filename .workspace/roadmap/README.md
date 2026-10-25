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
- 次の空き番号は目次の最大 ID + 1（いまは次 = **`x0011`**）
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

**現在の次:** （未定義 — x0010 完了。Grab hold は **x0011**）

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
