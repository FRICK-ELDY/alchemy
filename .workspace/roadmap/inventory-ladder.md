# Inventory ladder（B+2 → C+3）

host（実行時親子）と assets（永続同一性）を **交互・細 Slice** で進める目次。  
最終形は **C+3**（版管理つきアイテム＝VcsRepo）。最初の出荷定義は **B+2**（セッション跨ぎ個人インベントリ＋DriveNode 論理 Prefab）。

ID は roadmap 規約どおり **`x` + 4桁ゼロ埋め小文字 16進**（`x0059` の次は `x005a`）。

## 原則

| 守る | やらない |
|:---|:---|
| 1 Slice = 1 保証境界（下記表） | host だけ Step 3〜4、または assets だけ Drive/Vcs/Shop を先行 |
| Prefab は最初から論理 Tree（子 blake3 + TRS）。opaque 1 blob に閉じない | 最初から Vcs 全面／Shop／Mail |
| 片側だけ 2 Wave 相当以上走らない | XR ホットスイッチ（x0053）と同一 PR に混ぜる |

## Wave × Slice

| Wave | 到達 | Slice |
|:---|:---|:---|
| **0 権利楔** | 私有 blob を Grant 保有者だけが GET | [x0054](slices/x0054-inventory-grant-required-put.md) · [x0055](slices/x0055-item-ref-blake3-draft.md) |
| **1 箱庭・身体** | 「掴んでいる」が親子として表せる（未永続） | [x0056](slices/x0056-shared-sim-held-slot.md) · [x0057](slices/x0057-host-grab-held-slot.md) · [x0058](slices/x0058-vault-personal-peel.md) |
| **2 空間グラフ + Drive 薄皮** | 渡す／置く／undo + DriveNode | [x0059](slices/x0059-slot-graph-general.md) · [x005a](slices/x005a-slot-pass-place.md) · [x005b](slices/x005b-slot-session-undo.md) · [x005c](slices/x005c-drive-node-folder-blob.md) · [x005d](slices/x005d-drive-node-prefab-kind.md) |
| **3 B+2 保存** | 退出→再入室→取り出し | [x005e](slices/x005e-prefab-payload-schema.md) · [x005f](slices/x005f-inventory-take-out.md) · [x0060](slices/x0060-inventory-put-back.md) · [x0061](slices/x0061-inventory-rejoin-roundtrip.md) |
| **4 C+3 昇格** | Prefab→VcsRepo・`repo@rev` | [x0062](slices/x0062-prefab-promote-vcs.md) · [x0063](slices/x0063-item-ref-repo-rev.md) · [x0064](slices/x0064-vcs-branch-tag-minimal.md) · [x0065](slices/x0065-origin-cert-thin.md) |

## ラダー外（依存・後続）

| 対象 | 扱い |
|:---|:---|
| GET 常時 Grant（案 B 本体）・room 連動読取 Grant | x004f 後続。本ラダーの x0054 は **inventory PUT 固定**のみ |
| Mail / Comms / Shopfront | Drive・Vcs 安定後の独立帯（Wave 4 後に Shop） |
| CRDT・Formula VM | host Step 4〜5。B+2 / C+3 本線の外 |
| x0053 XR↔Desktop ホットスイッチ | 体感帯・並行可・同一 PR 禁止 |

## 関連

- 目標像: [assets-server-draft.md](../0_doc/architecture/assets-server-draft.md) · [overview.md](../0_doc/architecture/overview.md) Inventory タブ
- 現行 assets: [assets-server.md](../0_doc/architecture/assets-server.md)
- 現行 host/client: [client.md](../0_doc/architecture/client.md)

← [Roadmap 目次](README.md)