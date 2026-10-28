# x0038 — 検証失敗は破棄（leave→discard）+ 否定系 E2E

> **ID:** `x0038` · **slug:** `leave-to-discard` · **status:** `done` · **area:** `[client, docs, ci]` · **depends_on:** `[x0026, x001f, x0037]`

目標: Manifest / Presence の pin・seal・attest 失敗で `leave_room_local()` しない。到達 peer の偽 1 パケットで部屋が空にならない。失敗は drop + counter。退室は検証済み世代切替（または明示 Leave）のみ。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P0-1**

正本: [trust-boundary.md](../../0_doc/architecture/trust-boundary.md) · [client.md](../../0_doc/architecture/client.md)

## 設計

| 層 | 内容 |
|:---|:---|
| client | `protocol_pin` / `room_id` / `world_seal` / `host_attestation` / Presence pin・`host_sig` 失敗 → **apply せず drop**。5s 集約 log counter |
| 退室 | 明示 Leave（x0019）と、検証済み Manifest 世代切替（x003a 以降）のみ `leave_room_local` |
| Frame 整合 | 偽 Frame は既に破棄（x0023）。Manifest/Presence を同じ「破棄」契約に揃える |
| 文書 | trust-boundary の「失敗は leave」を「失敗は discard」に更新 |
| E2E | e2e-probe が偽 Manifest を publish → host が Frame 継続。client leave 回避は identity gate unit |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | client: leave→discard + counter | done |
| 2 | trust-boundary / client / protocol 文書 | done |
| 3 | 否定系（偽 Manifest）E2E | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 偽 Manifest / Presence 1 パケットで全 client が退室しない | Zenoh ACL / mTLS（→ P2） |
| 検証失敗サンプルは適用されない（旧検証済み状態を保持） | seal oracle・revision replay（→ x0039 / x003a） |
| 否定系 E2E が緑 | GUI client の体感 DoS 全網羅 |

## やらない

- canonical ManifestSeal（→ x0039）
- generation / revision ゲート（→ x003a）
- decode byte/count limit（→ x003b）
- transport ACL

## 成功条件

1. 偽 Manifest（pin/seal/attest のいずれか不正）を受信しても `leave_room_local` が走らない  
2. 直前の検証済み Manifest / Presence / Frame 経路が継続する  
3. trust-boundary が「失敗は discard」とコード一致  
4. 否定系 E2E（偽 Manifest）が緑  

← [目次](../README.md)
