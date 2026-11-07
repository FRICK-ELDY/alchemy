# x0069 — 部屋隔離（Room 鍵 AEAD）

> **ID:** `x0069` · **slug:** `room-key-aead` · **status:** `done` · **area:** `[protocol, world, host, client, zenoh, docs]` · **depends_on:** `[x004e, x0066, x0068]`

目標: WAN ACL のワイルドカード購読で `wire_cap` が列挙されうる前提を認め、**部屋の機密性を Room 鍵 AEAD（XChaCha20Poly1305）で担保**する。L7 プロキシ化はしない。

根拠: [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P0-3**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 正規 Room Token 保持者のみが manifest / frame / presence / snapshot / input の平文を読める | zenohd 上のトピック名（`wire_cap`）秘匿；wildcard subscribe 阻止 |
| AAD（`room_id` + kind）により別部屋・別チャネルへの暗号文差し替えを拒否 | 入室者同士の Room 鍵 forward 秘匿（参加者は共有） |
| 復号失敗は protobuf / 署名検証の前に discard | put 洪水 DoS；退室即時失効（**Room 鍵ローテは [x007e](x007e-room-key-epoch.md)**） |
| 送信ごと 192bit 乱数 Nonce（XChaCha20Poly1305）で送信者間 Nonce 衝突を構造回避 | L7 Zenoh プロキシ；QUIC；Allocation / Relay への `room_key` 配布 |

## 設計（要点）

- 第1壁（CN ACL）は現状維持。第2壁の機密性正本は **Room 鍵 AEAD**（`wire_cap` はキー式ルーティング用）
- world が room 作成時に 32B 鍵を発行し、Token JSON の `room_key`（URL-safe base64）で配布。Elixir 新規依存なし
- Zenoh payload = `nonce(24) || ciphertext||tag(16)`。AAD = `alchemy/aead/v1|{room_id}|{kind}`
- `PROTOCOL_PIN` = `alchemy-protocol/0.18.0-x0069`（LAN tcp も常時 AEAD）

## 完了条件

1. guest が他部屋の平文（manifest 等）を読めない（鍵なし / 別鍵 → AEAD discard）
2. AAD の `room_id` または kind を差し替えた暗号文は discard
3. 正規 host/guest は同一 room で Frame / Intent / Presence / Manifest が通る
4. 保証表（trust-boundary / relay / x004e / peer-delivery）が上記と一致
5. protocol / rust / world CI 緑

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 保証表・peer-delivery・roadmap | done |
| 1 | world `room_key` 発行・Token 配布 | done |
| 2 | protocol `wire_aead` + host/client 配線 + pin bump | done |
| 3 | 否定系・CI | done |

## 実装メモ

- Allocation JWT / Relay allocate 応答には `room_key` を載せない
- liveliness は payload 無しのため AEAD 対象外
- ciphertext 上限 = 既存 plaintext 上限 + 40

← [目次](../README.md)
