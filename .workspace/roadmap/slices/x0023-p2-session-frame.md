# x0023 — P2 後半（session binding / Frame 検証 / snapshot）

> **ID:** `x0023` · **slug:** `p2-session-frame` · **status:** `done` · **area:** `[protocol, host, client, docs]` · **depends_on:** `[x0022]`

目標: 2026-10-26 評価の **P2 後半**を塞ぐ。(1) バス購読だけで Room Token なりすましできない。(2) client が偽／壊れた Frame で NaN 描画せず、late join が収束する。(3) `PROTOCOL_PIN` 不一致を入室時に検出する。

根拠: [improvement-plan-2026-10-26.md](../../0_doc/evaluation/improvement-plan-2026-10-26.md)（P2-2 / P2-4 / P2-5）

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [client.md](../../0_doc/architecture/client.md)

## 設計方針

| ID | 層 | 内容 |
|:---|:---|:---|
| P2-5 | protocol / host / client | `SceneManifest` / `PresenceState` に `protocol_pin`。host は常に `PROTOCOL_PIN` を載せる。client は不一致なら適用拒否（ログ＋入室ゲート） |
| P2-2 | protocol / host / client | Intent は **Ed25519 セッション鍵**に紐付け。`room_token` は bind / Token refresh 時のみ。定常 Intent は `session_pub` + `intent_seq` + `intent_sig`。host は公開鍵→`(sub,jti,last_seq)`。盗聴 JWT だけでは偽造不可 |
| P2-4 | protocol / host / client | (a) Frame の finite / 単調 `tick` ゲート (b) host が `…/snapshot` queryable（全 Entity の `RenderFrame`）(c) 箱の毎 tick 全 upsert 廃止 (d) host Ed25519 で Frame 署名、`SceneManifest.host_pub` で client 検証 |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | P2-5 protocol_pin + host 載せ + client 拒否 | done |
| 2 | P2-2 InputEvent セッション鍵 + host gate + client 署名 | done |
| 3 | P2-4 snapshot queryable + 純デルタ + finite/tick/署名 | done |
| 4 | architecture / 単体成功条件・PIN | done |

## 契約

### Intent（P2-2）

| 経路 | 必須フィールド | host 動作 |
|:---|:---|:---|
| Bind / rebind | `room_token` + `session_pub`(32) + `intent_seq` + `intent_sig`(64) | JWT verify → **同一 `sub` の他 pub を削除**してから `session_pub → (sub,jti,last_seq)`（後勝ち・1 session/sub） |
| Steady | `session_pub` + `intent_seq` + `intent_sig`（`room_token` 空） | pubkey 既知・`seq > last_seq`・sig OK なら Intent 適用。JWT なし |
| Leave | 同上（bind でも steady でも可） | 検証後即 reclaim |
| Token refresh | 新 `room_token` + **同じ** `session_pub` + sig | jti 更新。client は put 成功まで Token 付きを再送；liveliness は bind put 後にだけ回転 |
| Host 世代交代 | Manifest `host_pub` が **Some→別の Some** | world リセット + **`IntentSession::new()`** + snapshot 再取得（`None→Some` 初回は鍵を捨てない） |
| Presence reclaim | 自 slot が **割当済み→欠落** | 同じ鍵で `mark_bind_pending`（初回未割当の待ちでは再武装しない） |
| Snapshot 成功 | `upserts` **非空** + 署名 OK | 空 frame は未準備（host は `reply_err`）。tick 単調ゲートは snapshot では適用しない。tick+entities は同一 `WorldView` ロック |

- 署名対象: `InputEvent` の prost encode（`intent_sig` を空にしたもの）
- `PROTOCOL_PIN`: `alchemy-protocol/0.9.0-x0023`

### Frame / Snapshot（P2-4）

| 経路 | 内容 |
|:---|:---|
| `…/frame` | 純デルタ。pose 変化時のみ upsert。`host_sig` = host 秘密鍵で encode（sig 空）を署名 |
| `…/snapshot` | queryable。応答は `RenderFrame`（その時点の全 Entity upsert、`removes` 空）+ 同じ `host_sig` |
| client | Manifest の `host_pub` で検証。非 finite Entity は捨てる。`tick <= last` は無視。join 時と大きな欠落時に snapshot get |

### PROTOCOL_PIN（P2-5）

- Manifest / Presence の `protocol_pin` が定数と一致しない → client は状態を適用しない（入室継続不可として扱う）

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| バスで拾った Room Token だけでは他人として Intent を送れない（秘密鍵が必要） | Zenoh ACL・mTLS・Token そのものの盗聴防止（TLS は x0022） |
| bind 時の JWT 窓で秘密鍵が漏れない限り、以降のなりすまし不可 | client 端末上の秘密鍵窃取・恶意 client |
| 偽／壊れた Frame（NaN・逆行 tick・不正署名）を描画に載せない | 完全な欠落補間・予測・jitter buffer |
| late join / host 再起動後に snapshot で entity 表が収束する（置換適用・ゴースト無し） | world 署名 Manifest・SceneManifest 駆動（P4） |
| 世代違い host/client を pin 不一致で検出し入室を解除する | buf breaking CI・後方互換マルチバージョン・host verify ack チャネル |

## やらない（後続）

- Quaternion を RenderFrame に載せる（P3-11）
- SceneManifest 駆動 / shared-sim（P4）
- P3 Quota / Parsers / audit まとめ
- assets BlobGrant（P1-5 中期）— 短期は architecture 保証表のみ追記可

## 成功条件

1. protocol / host 単体: Token のみ・鍵なしの Intent は steady で棄却；正しい鍵なら Token なしで通る  
2. host 単体: 偽 `session_pub` / 逆行 `intent_seq` / 壊れた sig を棄却  
3. client: pin 不一致 Manifest を適用しない；非 finite / 不正署名 / 旧 tick Frame を捨てる  
4. snapshot get 後、箱を毎 tick upsert しなくても late join で箱が見える  
5. `make ci`（rust/protocol）緑。Compose E2E が session Intent でも通過  

## 起動メモ

```bash
# 日常スタックは x0020 のまま（PIN だけ上がる）
zenohd -c zenoh-router/config.json5
# host-server → client（Desktop）

cd client && cargo test -p alchemy-protocol -p host-server -p client
```

← [目次](../README.md)
