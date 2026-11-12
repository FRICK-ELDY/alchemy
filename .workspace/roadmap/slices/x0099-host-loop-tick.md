# x0099 — HostLoop::tick 切り出し（P2-6 残り）

> **ID:** `x0099` · **slug:** `host-loop-tick` · **status:** `done` · **area:** `[host]` · **depends_on:** `[x0095, x0093]`  
> **優先度:** improvement **P2-6** 残り（pose ダブルバッファ done 後の tick 切り出し）

目標: host 60Hz 物理ループ本体を `HostLoop::tick` に切り出し、`main` は sleep / lateness / 起動配線だけにする。Zenoh なしで mid-net / resume catch-up / net-tick を単体テスト可能にする。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-6** · [x0095](x0095-wire-pose-double-buffer.md)「やらない／次」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 1 physics tick の副作用一式は `HostLoop::tick`（spawn/despawn → gate/manifest → reclaim/Presence → intent → `step_dt` → mid-net/net submit → 1Hz log） | boot / Zenoh / credential worker / input gate / liveliness 購読の `main` 残置を変えない |
| sleep・`next_physics`・tick lateness は `main` | PROTOCOL_PIN・空間 UI・XR loader tempfile・bind 上限 |
| mid-net `removes_only` / resume catch-up latch / net-tick acquire→fill→submit は単体で回帰可能 | pose プール再設計（x0095 契約維持） |
| `PresenceIntents` は `presence` モジュール（入力と tick の共有型） | input gate 本体の大規模再配置 |

---

## 設計（要点）

```
main: sleep / lateness / shutdown
        │
        ▼
HostLoop::tick(now)
  spawn/despawn inbox
  publish gate / manifest / join-assist
  reclaim → release_slot / session GC
  apply intent/grab → physics.step_dt
  mid-net removes_only | net acquire/fill/submit
  1Hz diagnostic log
```

- `presence.rs`: `PresenceIntents` / `PoseOverride` + TTL/grace 定数
- `host_loop.rs`: `HostLoop` + wire pose fill + publish gate helpers

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | `presence.rs` 切り出し + main 接続 | **done** |
| 2 | `HostLoop::tick` + main は sleep のみ | **done** |
| 3 | 単体テスト + docs / P2-6 done | **done** |

---

## 完了条件

1. `cargo test -p host-server` 緑
2. `main` の物理 `loop` が sleep + `host.tick`（+ shutdown）であること
3. mid-net reclaim → `removes_only` の回帰テスト
4. gate off→on で `resume_catchup` の回帰テスト
5. net-tick が `acquire_poses` → fill → `submit_frame` を通る回帰テスト
6. PROTOCOL_PIN / 空間 UI を触っていないこと

## やらない（本 Slice）／次

- XR loader tempfile（P2-12）· bind 全体上限（P2-14）· USER_ROOT 帯 Manifest（P2-13）
- Inspector / Uber-SDF / 親子同期（空間本線）

## 関連

- [x0095-wire-pose-double-buffer.md](x0095-wire-pose-double-buffer.md)
- [x0093-xpbd-wire-authority.md](x0093-xpbd-wire-authority.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
