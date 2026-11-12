# x0095 — wire pose ダブルバッファ置換（P2-6 残り）

> **ID:** `x0095` · **slug:** `wire-pose-double-buffer` · **status:** `done` · **area:** `[host]` · **depends_on:** `[x008f, x0093]`  
> **優先度:** improvement **P2-6** 残り（空間本線は一時停止）

目標: host net-tick の wire Frame 用 pose `Vec` を、physics ↔ `zenoh-publish` 間の **scratch 返却（ダブルバッファ）** で回し、定常経路での毎 tick `Vec::with_capacity` をやめる。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-6** · [x008f](x008f-host-frame-bridge.md) / [x0093](x0093-xpbd-wire-authority.md)「やらない／次」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| net-tick pose 用 `Vec<(u32, Pose)>` は起動時確保＋返却で回す（fill+pending+worker の 3 本で定常経路は毎 tick `with_capacity` しない） | Frame protobuf / AEAD / Zenoh put 自体のゼロアロケ；3 本超の病的遅延時の追加確保 |
| coalesce で捨てた pose バッファも free へ戻す | removes リストや Entity upserts のプール化 |
| mid-net `removes_only` は空 poses のまま（プール取得しない） | host `main` 物理ループ全体の `HostLoop::tick` 切り出し |
| `FrameSample` は非 `Clone`（poses のプール契約すり抜け防止） | |
| coalesce / catch-up / empty-hold 契約は維持 | PROTOCOL_PIN・空間 UI（Inspector / Uber-SDF / 親子同期） |

---

## 設計（要点）

```
physics net tick
  outbound.acquire_poses() → clear → fill (mesh / user_root / boxes)
  submit_frame(FrameSample { poses, ... })
        │
        ├─ coalesce で捨てた poses → PoseScratchPool.release
        └─ zenoh-publish handle_frame 後 → release
```

- `OutboundHub` に `PoseScratchPool`（起動時 **3** 本 = fill + pending + worker）。枯渇時のみ追加確保（設定 capacity）；プール上限超過の返却は drop。
- wire fill は `fill_wire_poses_from_sim`（`clear` 後に append）。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | `PoseScratchPool` + coalesce recycle + worker release | **done** |
| 2 | physics `acquire`→fill→`submit` 配線 | **done** |
| 3 | 単体テスト + `cargo test -p host-server` | **done** |

---

## 完了条件

1. `cargo test -p host-server` 緑
2. net-tick 定常経路が `acquire_poses` → fill → `submit_frame` であること
3. coalesce で捨てた pose バッファが free に戻る回帰テスト
4. release 往復で `capacity()` が消えない回帰テスト
5. mid-net `removes_only` がプール取得しないこと（`free_len` 不変の回帰）
6. 空間 Lens / PROTOCOL_PIN を触っていないこと

## やらない（本 Slice）／次

- Inspector / Uber-SDF / host↔client 親子 / TPS・Live（空間本線は一時停止）
- `HostLoop::tick` 切り出し全体
- improvement P2 の他残件（XR loader tempfile 等）。take-out session は [x0098](x0098-take-out-owner-session.md)；Origin Cert hash は [x0096](x0096-origin-cert-content-hash.md)

## 関連

- [x008f-host-frame-bridge.md](x008f-host-frame-bridge.md)
- [x0093-xpbd-wire-authority.md](x0093-xpbd-wire-authority.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
