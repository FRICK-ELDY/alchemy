# x0045 — host Room Token 否定系テスト

> **ID:** `x0045` · **slug:** `room-token-negative-tests` · **status:** `done` · **area:** `[host, docs]` · **depends_on:** `[x003e, x0040]`  
> **優先度:** **P3-8**

目標: `RoomTokenVerifier::verify` の alg / iss / aud / exp / room（＋ unknown kid / invalid role）拒否を単体テストで固定する。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P3-8**

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 不正 alg（none / HS256）・iss・aud・exp・room_id の verify 失敗テスト | E2E（x0046）；fuzz は [x004c](x004c-cargo-fuzz-protobuf-jwt.md) |
| unknown kid / invalid role の拒否テスト | 本番 HTTP JWKS 経路の結合 |

## 完了条件

- `cargo test -p host-server verify_rejects` 緑

← [目次](../README.md)
