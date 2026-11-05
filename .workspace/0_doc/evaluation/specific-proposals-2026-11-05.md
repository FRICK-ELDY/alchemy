# Alchemy — 提案（0点）統合一覧（まとめ / 2026-11-05）

> 根拠: [Opus](opus/opus-specific-proposals-2026-11-05.md) · [GPT](gpt/gpt-specific-proposals-2026-11-05.md)  
> 減点せず「次のステップ」として記録。新規依存は人に回す。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 実装すれば価値が上がる提案 |

---

## P0 に直結する設計提案

- **制御面を session 鍵署名に統一 + world host API に PoP** `0`（Opus）
  > Intent と同じく Token は bind 時のみ。`host_keys` / seal / scene に `cnf.spk` 署名ヘッダ（DPoP 簡易版）。新規依存不要。
  > 対象ファイル: `client/app/src/inventory.rs`, `world-server/lib/world_web/controllers/room_controller.ex`

- **Room key epoch / revoke-driven rotation** `0`（双方）
  > visibility 縮小・kick・定期で回転。AAD + Token に epoch。猶予 = Token TTL。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `protocol/src/wire_aead.rs`

- **Scene revision 楽観 CAS** `0`（GPT）
  > `expected_revision` + `UPDATE … WHERE revision=?`。host は 409 で再取得再試行。
  > 対象ファイル: `world-server/lib/world/rooms.ex`

---

## 門番・資源

- **relay-agent CSR-only 化または退役** `0`（双方）
- **Allocation 主体別 throughput + Room Token 操作別 RL** `0`（双方）
- **GLB decode budget 共通型** `0`（GPT）
- **Port プール化 + `/ready`** `0`（Opus）
- **mTLS を room 束縛（長期）** `0`（Opus）

---

## テスト・運用

- **実 host-server を通す WAN 一往復 E2E** `0`（双方）
- **host 乗っ取り連鎖の否定系** `0`（Opus）
- **assets test の hermetic 化 / Windows XR loader tempfile** `0`（双方）
- **`ci.sh` に shared Elixir 3 パッケージ** `0`（Opus）
- **SLO dashboard（loopback metrics 集約）** `0`（GPT）
- **proptest / StreamData・protocol fuzz paths** `0`（Opus・人に回す）

---

## 目標機能（減点対象外）

- **Verifier Spot-Check orchestration（Noop 配線）** `0`
- **Frame 補間 / jitter buffer** `0`
- **launcher deep link / host handoff** `0`
- **5 人以上の部屋（USER_ROOT 帯を Manifest 配）** `0`
- **billing / market / Relay QoS 経済圏** `0`（ディレクトリ未設置）
