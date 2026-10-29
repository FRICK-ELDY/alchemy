# x0046 — adversarial E2E（偽 Manifest / stale-rev / oversize）

> **ID:** `x0046` · **slug:** `adversarial-e2e` · **status:** `in_progress` · **area:** `[ci, host, protocol, docs]` · **depends_on:** `[x0038, x003a, x003b, x001f]`  
> **優先度:** **P3-1**

目標: hermetic compose 上で偽 Manifest・seal 無し stale-rev・oversize payload を Zenoh に投げても host が Frame を出し続けることを e2e-probe で固定する。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P3-1**

「compose fault」= docker chaos ではなく、hermetic compose スタックへの adversarial put（[`smoke-compose-e2e.sh`](../../../.scripts/smoke-compose-e2e.sh) 経由）。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 偽 Manifest / seal 無し stale-rev / oversize Input 後も host が **新規** Frame を出し続ける | 署名付き旧 seal の GUI discard（→ x003a 単体正本） |
| Input oversize が host `check_payload_bytes(WireClass::Input)` を踏む | peer Manifest の client discard（host は peer Manifest を subscribe しない） |
| oversize Manifest を Zenoh に put した後も host プロセスが Frame を出し続ける（大ペイロード後の生存） | R2 / Zenoh disconnect / unknown kid / cargo-fuzz（→ 後続 P3） |

## 完了条件

- `e2e-probe` が drain→put→新規 Frame で fake + stale-rev + oversize を assert
- compose smoke 経路で緑（既存 `.scripts/smoke-compose-e2e.sh`）— **未実行なら status は `in_progress`**

← [目次](../README.md)
