# x004e — Relay Agent MVP（P2-1）

> **ID:** `x004e` · **slug:** `relay-agent-mvp` · **status:** `done` · **area:** `[zenoh, relay, protocol, world, host, client, docs]` · **depends_on:** `[x0022, x003e, x003b]`  
> **優先度:** **P2-1**

目標: WAN 上で **Allocation Token → 短命 mTLS → role ACL + `wire_cap` キー隔離** を入れ、任意 peer が他 room の `manifest` に書けないようにする。課金・第三者リレー・CCU QoS は後続。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P2-1**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| WAN: mTLS 未提示 peer は zenohd に入れない | Billing / 70:30 / 第三者リレー発見 |
| Allocation Token 検証後にだけ短命 client 証明書を渡す | CCU・帯域 Token Bucket enforcement |
| guest CN は `manifest`/`frame`/`presence`/`snapshot` に put 不可；host CN は `input` に put 不可 | Zenoh ACL ホットリロード / 動的 per-room subject |
| `wire_cap` を知らない peer は他 room キー式に到達できない | `alchemy/tenant/{運営者}` 本番名前空間への全面移行 |
| ローカル e2e は tcp・mTLS オフのまま（opt-in WAN） | Relay Agent の L7 Zenoh プロキシ化 |

## 設計（要点）

- Zenoh ACL は実行時更新不可 → **role 固定 CN**（`alchemy-host` / `alchemy-guest`）+ 静的キー族 ACL
- room 隔離はキー式の **`wire_cap`**（world が room 作成時に発行）
- Allocation Token: world 発行、`aud=alchemy-relay`、既存 JWKS
- Relay Agent: `POST /v1/allocate` → openssl で短命 cert（新規 Cargo crypto crate は使わない）

## 完了条件

1. 偽 Allocation / mTLS なし → allocate または Zenoh open 失敗
2. guest cert で manifest put 不可
3. 未知 `wire_cap` では他 room manifest に書けない
4. 正規 host/guest は同一 room で Frame / Intent / Presence が通る
5. ローカル e2e（tcp）緑

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | protocol `wire_cap` + pin bump | done |
| 2 | world wire_cap + Allocation Token | done |
| 3 | relay-agent + WAN mTLS/ACL config | done |
| 4 | zenoh-connect / host / client 配線 | done |
| 5 | 否定系・手順書・e2e | done |

## 実装メモ

- 証明書 CN は静的 ACL のため **`alchemy-host` / `alchemy-guest` 固定**（room は `wire_cap`）
- ローカル e2e: `RELAY_ALLOCATE_URL` 未設定なら mTLS スキップ（tcp）
- **Home→Join（ローカル含む）**: `bind_room_after_join` が room bus（spawn_*）を起動。mTLS は `mtls_reopen_pending` ワンショットで reopen（env 存続では再 open しない）
- **Leave / bus 破棄は `session.close().wait()`**（tcp/WAN 共通）。Zenoh 1.x は strong_counter>1 だと Drop では close しない → 旧 input が Token 復活で二重 Intent になるのを防ぐ
- `ensure_room_bus` 失敗時は flag 巻き戻し；bind 経路は close+reopen で orphan 掃除後リトライ
- allocate `expires_in=300`（再取得 SLA）。openssl NotAfter は `-days 1`（ツール下限）。完了条件の「未知 wire_cap」は推測不能依存
- host Token refresh で allocation を再取得し cert ファイルを更新（稼働中 session の再 open は未自動化・NotAfter≈1d）。guest refresh は re-allocate しない

← [目次](../README.md)
