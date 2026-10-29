# x0042 — Zenoh network worker（P1-3 残り）

> **ID:** `x0042` · **slug:** `zenoh-network-worker` · **status:** `done` · **area:** `[host, protocol, docs]` · **depends_on:** `[x003d, x0024, x0023]`  
> **優先度:** **P1-3 残り** — 物理ループから Zenoh `put().wait()` を外す

目標: Manifest / Presence / Frame の同期 Zenoh publish を専用 `zenoh-publish` スレッドへ移す。物理はワイヤ非依存の pose / 配信ジョブを bounded スロットへ投げるだけ。encode（現状 protobuf）は network 側に閉じ込め、将来の rkyv + bytemuck 差し替えに備える。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P1-3** · [x003d](x003d-credential-worker.md)（credential 済み・put は非保証だった）

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 物理ループに Frame / Manifest / Presence の `put().wait()` が無い | protobuf → rkyv 実装そのもの |
| Frame 投入は pose + removes 等の論理サンプル（`RenderFrame` をチャネルに載せない） | Prometheus 本格（→ P3-4） |
| 満杯時 Frame coalesce: **poses は latest**、**`removes` は和集合**、**`resume_catchup` は sticky OR**（物理はブロックしない）。ゲート中サンプルは removes/catch-up を worker 内 defer | Zenoh ACL / mTLS（→ P2-1） |
| 30Hz・empty hold（put 無し / `net_tick` 非進）・gate / resume catch-up は従来どおり | Intent 受信スレッドの再配置 |
| encode + sign は network worker 内（今日は prost）。`put` 失敗は `exit(78)` | Manifest ワイヤの即時非 protobuf 化（seal 済み Bytes 透過） |
| — | `stop()` 後の join / pending ドレイン（プロセス終了で in-flight put が落ちうる） |

## 設計

1. **`OutboundHub`**（slice 旧称 OutboundSlots）: Frame / Manifest / Presence 各1スロット + wake。Frame は `coalesce_frame`
2. **物理**: net tick / mid-net removes / presence・manifest タイミング判定のみ。pose サンプル投入
3. **`zenoh-publish`**: `last_sent`・`net_tick`・delta・sign・put・`snapshot_cache` 更新
4. **メトリクス**: coalesce / publishes / empty_hold を atomic + 既存 1s ログ

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | network_worker + main 配線 | done |
| 2 | テスト + architecture / improvement-plan | done |

## 完了条件

- 物理経路に三 publisher の `.put().wait()` が残っていない
- coalesce / empty-hold / catchup の単体または回帰テストが緑
- `cargo test -p host-server` / clippy `-D warnings` 緑
- P1-3（credential + network）を improvement-plan 上閉じる

← [目次](../README.md)
