# x0070 — headless client trust + WAN 否定系 E2E

> **ID:** `x0070` · **slug:** `headless-trust-wan-e2e` · **status:** `done` · **area:** `[client, ci, zenoh, docs]` · **depends_on:** `[x0038, x003a, x0046, x0066, x0069]`

目標: GUI と同じ Manifest gate を使う headless consumer で偽 seal 否定系を hermetic Compose に固定し、WAN（mTLS + `config.wan`）上で wildcard 購読でも他室平文が読めないことを CI 固定する。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P1-3** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P1-2**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| headless が app と同一 gate（AEAD → identity → seal → attest → gen-rev）で偽 seal / wrong-key / oversize を Discard | GUI/wgpu、明示 Leave の体感 |
| Discard 後も joined 相当の watermark（`host_pub` / gen-rev / `last_world_seal`）が不変 | 署名付き旧 seal JWT の全マトリクス（unit 正本） |
| WAN: guest が `*/*/manifest` を wildcard subscribe しても、**他室平文は読めない**（AEAD discard） | `wire_cap` 秘匿・サンプル未達・L7 部屋購読強制 |
| 未提示 / 偽 client cert → WAN TLS 握手拒否（第1壁の最小固定） | guest sim put deny の網羅（x0067 正本） |

## 設計（要点）

- `client/trust`（`alchemy-client-trust`）に JWKS / seal / attest / identity gate / `process_manifest_*` を集約。Leave は返さない
- hermetic: 既存 tcp zenohd 上で `trust-probe` が正規 Accept → 敵対 put → watermark 不変
- WAN: compose overlay で `config.wan` + CI 生成 cert + world `MTLS_CA_*`。主 assert は **ciphertext unreadable**（未達ではない）

## 完了条件

1. 偽 seal / wrong-key AEAD を実 trust gate が Discardし、joined watermark 不変 — hermetic CI 緑
2. WAN compose で wildcard 他室暗号文が自室鍵で読めない — CI 緑
3. 未提示 cert の WAN 接続失敗 — CI 緑
4. 保証表（trust-boundary / 本スライス）が「未達」ではなく「読めない」と一致
5. Rust fmt/clippy/test + e2e 緑

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / backlog | done |
| 1 | `client/trust` 抽出 + app 配線 | done |
| 2 | hermetic trust-probe + smoke | done |
| 3 | WAN compose + wildcard / 無証拒否 CI | done |

← [目次](../README.md)
