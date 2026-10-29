# x004c — BLAKE3 ストリーミング・Room 作成上限

> **ID:** `x004c` · **slug:** `blake3-stream-room-limits` · **status:** `done` · **area:** `[assets, world, docs]` · **depends_on:** `[x0028, x003f]`  
> **優先度:** **P3-7**

目標: アップロード BLAKE3 から **内容 tempfile** を外し（Unix=FIFO）、Room 作成に **per-owner 件数上限** と **作成レート制限** を付ける。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P3-7**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Unix: `Assets.Blake3` は FIFO 経由で `b3sum`（**内容 tempfile なし**；FIFO ノードは作る）。writer/reader は unlink + 共有 deadline（timeout でも親は生存）。Windows は短命 content tempfile | Rustler NIF；Windows FIFO 同等；timeout 時 OS 子 `b3sum` の一瞬 orphan（init 回収） |
| owner あたり `max_rooms_per_owner`（既定 32）超過の create 拒否 | 厳密な並行上限（check-then-insert の競合で +α はあり得る） |
| create の固定窓レート制限（既定 owner 10 / 60s）。**insert 前に消費**（失敗でも枠消費）。429 に `Retry-After` | IP 軸・グローバル総部屋数・削除後の履歴課金；成功時のみ課金 |

## 実装メモ

- BLAKE3: body はメモリ上。弱さは「毎回 content tempfile + CLI」。Unix は FIFO（内容はディスクに乗らない）。Elixir `System.cmd` に stdin が無いため Port EOF / NIF は後段。
- Room: `World.Rooms.create/3` 内で count + `World.RateLimit`。HTTP は `owner_room_limit`（403）/ `rate_limited`（429 + Retry-After）。

## 完了条件

- blake3: 内容 tempfile なし（Unix）・既知 hex ベクトル緑・writer/reader timeout が親を落とさない
- rooms: 上限超過・rate limit の単体 / controller テスト緑（Retry-After 含む）
- architecture / README / improvement-plan が x004c を指す（用語は FIFO / 内容 tempfile）

← [目次](../README.md)
