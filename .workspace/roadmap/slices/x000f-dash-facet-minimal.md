# x000f — Dash Facet 最小

> **ID:** `x000f` · **slug:** `dash-facet-minimal` · **status:** planned · **area:** `[client, system_ui]` · **surface:** `[desktop, vr]` · **depends_on:** `[x0007, x000e]`

目標: Resonite Facet に近い **個人ダッシュ**の器を切る。UI ランタイム（Canvas / Button・hover・press）は将来の World UI と共通化しつつ、**所有・同期境界は `DashRoot`（個人・非公開）と `WorldRoot`（空間・共有）で分ける**。本 Slice では公式デフォルト Facet＋**Pinned Exit**＋Button ホバー色まで。並べ替え・ユーザー Facet 追加は後続。

正本: [client.md](../../0_doc/architecture/client.md) · [overview.md](../../0_doc/architecture/overview.md)（system_ui / Dual は目標）

## 設計方針

| 層 | 役割 |
|:---|:---|
| UI ランタイム（共通） | Canvas / Graphic / Button（hover・press・色）。OpenXR / Room を知らない |
| `DashRoot` | 個人コンテナ。Room の `frame` に載せない。他人は触れない |
| Facet | ダッシュ上のパネル単位。公式セットをコード定義で載せる |
| Pinned | Exit など必須 Facet。削除・スロット外し不可 |
| surface（Desktop / XR） | ピクセル座標の pointer／レーザー UV を渡すだけ。system_ui は XR 非依存維持 |
| `WorldRoot` | **本 Slice では触らない**（同じランタイムを後で載せる予約） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | 契約: `DashRoot` / Facet / Pinned + UI ランタイム境界を docs / モジュールコメントで固定 | pending |
| 2 | Button interactable: hover / press + 色（既存 hit 経路） | pending |
| 3 | XR レーザー命中 → hover（surface が UV を渡す。system_ui は XR 非知） | pending |
| 4 | 現行ダッシュを「公式デフォルト Facet セット」として Facet 語彙へ寄せる（見た目・Exit 経路維持） | pending |
| 5 | smoke + client.md / README 現状更新 | pending |

## 共通化するもの / しないもの

| 共通（1 系統） | 分けたまま |
|:---|:---|
| Canvas / Graphic / Button と hit・スタイル | Container: `DashRoot` vs `WorldRoot` |
| 「パネル＝コンポーネントの集まり」という編集モデル（後続で本格化） | Persistence: ユーザー設定 vs Scene Commit |
| 将来の編集ジェスチャの感覚 | Protection: Pinned Facet |
| | 同期: Dash 変更は Room に乗らない |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| ダッシュは個人・非同期（他クライアントから触れない） | ワールド共同編集・Room への Dash レプリカ |
| Exit は Pinned（消せない）。既存 Exit 経路を壊さない | ユーザー Facet の自由追加・並べ替え・非必須削除 UI |
| Button hover で色が変わる（Desktop ポインタ / XR レーザー命中） | Vault 永続のレイアウト保存、テーマエンジン |
| system_ui が OpenXR / Zenoh Room を知らないこと | `WorldRoot` 実装、Grab 編集、Formula / Dual Editor |
| Desktop ESC / XR Menu の開閉とレーザー（x000e）維持 | CompositionLayerQuad 復帰、WSL 実 HMD |

## やらない（後続）

- Facet スロットのドラッグ並べ替え・インベントリからの取り付け
- ユーザー自作 Facet / 非必須公式 Facet の削除
- Dash レイアウトのディスク／Vault 永続化
- `WorldRoot` 上の同一ランタイム実装（ワールドと同じ感覚の本格編集）
- system_ui への XR 知識流入

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server
# Desktop
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client
# Windows + SteamVR（WSL は HMD 非保証）
cargo run -p client
```

確認（Phase 完了時）:

1. ESC / Menu でダッシュ開 → Exit 等にポインタ／レーザーを当てると色が変わる  
2. Select / クリックで既存どおり Exit 動作  
3. 他クライアントや host の Scene にダッシュ Entity が増えない  

## 実装メモ（予定）

- `client/app/src/system_ui/` — Facet / Button hover。OpenXR 非依存維持
- `client/app/src/surface/` — Desktop マウス UV・XR レーザー UV を `on_hover` / `on_click` に渡す
- 現行 `dash.rs` / `layout.rs` は公式デフォルト Facet セットへ段階移行（ビッグバン置換しない）
- smoke: `bash .scripts/smoke-system-ui.sh` · `bash .scripts/smoke-openxr.sh`
- 先行: [x0007-system-ui-dash.md](x0007-system-ui-dash.md) · [x000e-vr-ui-laser-pointer.md](x000e-vr-ui-laser-pointer.md)

← [目次](../README.md)
