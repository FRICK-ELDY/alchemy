# x0087 — 実 host WAN 一往復 E2E

> **ID:** `x0087` · **slug:** `wan-host-e2e` · **status:** `in_progress` · **area:** `[client, ci, zenoh, docs]` · **depends_on:** `[x0070, x0066, x0069, x003d]`  
> **優先度:** 評価 P1-4（credential worker〜署名 Frame まで WAN で固定）

目標: WAN Compose 上で **実 host-server** が world CSR → credential worker → mTLS Zenoh → sealed Manifest/Frame を出し、headless trust consumer が Accept する一往復を CI 固定する。否定系は no-cert / guest sim put の peer 非配送 / 旧 epoch AEAD Discard。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P1-4** · [gpt-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/gpt/gpt-specific-weaknesses-2026-11-05.md)（WAN が synthetic Manifest 止まり）· [x0070](x0070-headless-trust-wan-e2e.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| WAN: host-server が CSR/mTLS で接続し、credential worker 経由の sealed Manifest を guest trust gate が **Accept** | GUI/wgpu、Tunnel、R2 |
| WAN: 同経路の sealed **Frame** を guest が AEAD open + decode できる（待ちは `E2E_TIMEOUT_SECS`） | Frame 内容の物理正しさの網羅 |
| WAN: client cert 無し → TLS 接続失敗（x0070 継承） | |
| WAN: guest CN の **sim put ingress** は peer guest に届かない（`put.wait` Ok でも配送されない） | put API が Err を返すこと；全キーマトリクス（x0067 正本） |
| 旧 `room_key_epoch` で seal した envelope を **ローカル** `process_manifest_bytes` が **AEAD Discard**（watermark 不変） | host B 経由で wire に旧 epoch を載せる経路；Token 回転 Compose |
| WAN: wildcard 他室暗号文が自室鍵で読めない（x0070 継承） | `wire_cap` 秘匿・L7 購読強制 |

---

## 設計（要点）

| 軸 | 内容 |
|:---|:---|
| Compose | 既存 `docker-compose.e2e.yml` + `docker-compose.e2e.wan.yml`（変更は保証コメント） |
| Smoke | `.scripts/smoke-compose-wan-e2e.sh` が **host-server** を起動；probe 中も `HOST_PID` 監視 |
| Probe | `TRUST_PROBE_MODE=wan`: Accept Manifest/Frame → guest sim put peer 非配送 → **local** old-epoch Discard → wildcard |
| Host | `should_request_world_mtls()`（`tls/`）で CSR；world overlay の CA が署名 |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / backlog / improvement | **done** |
| 1 | smoke に host-server 起動 + PID 監視 | **done** |
| 2 | trust-probe WAN を実 host 一往復 + 否定系に拡張 | **done** |
| 3 | 保証表・compose / workflow コメント更新 | **done** |
| 4 | GitHub Actions `wan-host-e2e` 緑 → status `done` | **pending** |

---

## 完了条件

1. WAN smoke が host-server を起動し、trust-probe が Manifest Accept + Frame open に成功
2. no-cert / guest sim put が peer に届かない / 旧 epoch Discard（local）が同一 probe で緑
3. wildcard 他室 unreadable（x0070）維持
4. architecture / improvement P1-4 更新；`wan-host-e2e` job 緑で status `done`

## やらない（本 Slice）

- Allocation 主体別 throughput（P1-5）
- hermetic の偽 seal 行列を WAN ACL 上で再実施（guest put 不可）
- 旧 epoch を host B セッション経由で wire に載せて Discard する経路（trust gate は local で十分）
- Spot-Check / market

## 関連

- [x0070-headless-trust-wan-e2e.md](x0070-headless-trust-wan-e2e.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
