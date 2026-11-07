# x007e — Room key epoch / revoke-driven rotation

> **ID:** `x007e` · **slug:** `room-key-epoch` · **status:** `done` · **area:** `[protocol, world, host, client, docs]` · **depends_on:** `[x0069, x0034, x007c]`  
> **優先度:** 評価 P0-2（Room 鍵寿命 = 公開範囲の実効寿命）。x007c KeyStale 本採用の前提

目標: `room_key` に **epoch** を与え、visibility 縮小・kick（owner 操作）・定期ローテで鍵を回転する。AAD と Room Token に epoch を束縛し、旧 epoch の平文は新鍵では読めないことを保証する。猶予窓は **現行 + 直前 1 世代**（≈ Room Token TTL）。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P0-2** · [opus-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-05.md)（Room 鍵が作成時一度きり）· [x0069-room-key-aead.md](x0069-room-key-aead.md) 非保証「Room 鍵ローテ」を本 Slice で閉じる · [x007c](x007c-worlds-catalog-join-host.md) KeyStale

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Room に単調増加 `room_key_epoch`（u64）と現行 `room_key`；Token JSON に `room_key` + `room_key_epoch` | Allocation JWT / Relay への `room_key` 配布（x0069 どおり載せない） |
| visibility **縮小**・owner kick（本 Slice で定義する経路）・定期ローテで epoch+1 と新鍵を発行 | 退室単独での即時全参加者失効（Token TTL 猶予は残る） |
| AEAD AAD に epoch を含める（差し替えで旧暗号文は新鍵で開かない） | 参加者同士の鍵 forward 秘匿（共有鍵モデルは x0069 のまま） |
| host は **現行 + 直前 1 世代**の鍵だけで open；それ以外は discard | 3 世代以上の受理；クライアント側マルチ鍵バッファの長期保持 |
| client は Token 再取得で epoch 前進を検知 → **KeyStale**（再 Join / Leave） | KeyStale 中の透過的な無停止鍵切替（必ず再 Token） |
| 保証表（world-server / client / peer-delivery / x0069 追記）に鍵寿命を明記 | Group ACL / 招待コード；WAN ACL のキー式縮小本体（別帯） |

---

## 設計（要点）

### データ（world）

- `rooms.room_key_epoch`（integer, default 0）+ 既存 `room_key`（現行のみ永続）
- 直前世代は **メモリ短命**（rotate 時に `prev_key` + `prev_epoch` + `prev_expires_at`；TTL ≈ max Room Token TTL）。永続しない（再起動後は現行のみ → 旧 Token 保持者は再発行が必要）
- Token 応答: 既存 `room_key`（b64）に加え `room_key_epoch`（number）
- **host** Token のみ: grace 中は `room_key_prev` + `room_key_prev_epoch`（`RoomKeyGrace.get`）を同梱。guest には載せない
- host は soft TTL に加え **≈15s 周期**で host Token を再取得し world rotate を採用（完了条件 2 の Rejoin 窓を Token TTL 全長にしない）

### ローテ契機

| 契機 | 動作 |
|:---|:---|
| `set_visibility` で ACL が狭まる（例: public→friends / private） | epoch+1 + 新鍵 |
| owner kick（本 Slice で API を定義。未実装なら Phase で「visibility 縮小と同等の revoke 入口」に落とす） | 同上 |
| 定期（設定: 既定 24h、`ROOM_KEY_ROTATE_HOURS`） | 同上 |
| visibility **拡大**のみ | ローテしない（任意・明記） |

### ワイヤ（protocol）

- AAD を `alchemy/aead/v2|{room_id}|{epoch}|{kind}` に更新（**PROTOCOL_PIN bump**）
- `seal` / `open` は呼び出し側が epoch を渡す；host open は current 失敗時に prev を試す（順序: current → prev）
- LAN tcp も常時 AEAD（x0069 どおり）

### host

- credential / Room Token 適用時に `(room_key, epoch)` を保持；rotate 通知は **再 Token**（world が新鍵を配る）か、host 自身が owner Token で再取得
- 配信は常に **現行 epoch** で seal
- 受信 Intent 等は current→prev の dual open

### client

- Join Token の epoch を保持；refresh で epoch が進めば KeyStale（**live AEAD は旧鍵のまま**；新鍵は Rejoin で commit）
- guest は soft TTL に加え **≈15s Token probe** と **AEAD open 失敗時の refresh nudge** で host の epoch 採用に追従（FRAME_STALL より先に KeyStale）
- KeyStale UI（x007c）: 「部屋の更新あり・再接続が必要」→ Rejoin（新 Token）/ Leave
- remount / bus は Leave→Join と同じ世代ゲートで捨てる（x007d `remount_generation`；KeyStale 入りでも bump）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 / backlog・x007c 依存追記 | **done** |
| 1 | world: 列 + rotate API/内部 + Token に epoch；縮小 visibility で回転；単体 | **done**（kick API は `rotate_room_key/2` 入口；定期は Phase 4） |
| 2 | protocol: AAD v2 + pin bump；単体否定系（旧 epoch 鍵で新暗号文不可） | **done**（pin `0.19.0-x007e` · world pin / demo vector 同期） |
| 3 | host dual-key open + 現行 seal；client Token epoch + KeyStale UI | **done**（`SharedRoomAead` current→prev · KeyStale Rejoin/Leave） |
| 4 | 定期ローテ・保証表・peer-delivery・否定系 E2E / CI | **done**（`RoomKeyRotator` + docs；否定系は protocol/host 単体。専用 E2E CI は任意フォロー） |

---

## 完了条件

1. 一度入室した guest が、visibility 縮小（または kick / 定期）後に **旧鍵のまま** 新 epoch の Manifest/Frame 平文を読めない（単体または E2E 否定系）。
2. 正規参加者は新 Token（新 epoch）で再 Join すると Live に戻れる。
3. host は Token TTL 相当の猶予で旧 Intent を受理し、猶予後は discard。
4. client が epoch 前進を KeyStale として示し、ログ無しでも再接続が必要と分かる。
5. architecture / x0069 追記 / peer-delivery が「鍵に寿命がある」と一致する。

---

## やらない（本 Slice）

- 制御面 Room Token 排除 / session PoP（improvement **P0-1**・別 Slice）
- Scene revision CAS（**P0-3**）
- relay-agent CSR-only（**P0-4**）
- WAN mTLS の room 束縛 L7（P3）

## 関連

- [x0069-room-key-aead.md](x0069-room-key-aead.md)
- [x007c-worlds-catalog-join-host.md](x007c-worlds-catalog-join-host.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)
- [world-server.md](../../0_doc/architecture/world-server.md) / [client.md](../../0_doc/architecture/client.md)

← [目次](../README.md)
