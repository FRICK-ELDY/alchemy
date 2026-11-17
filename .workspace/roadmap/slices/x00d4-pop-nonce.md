# x00d4 — HTTP PoP nonce（P2-2 完了）

> **ID:** `x00d4` · **slug:** `pop-nonce` · **status:** `done` · **area:** `[protocol, world, host, docs]` · **depends_on:** `[x0081, x00d2]`  
> **優先度:** improvement-plan **P2-2**（残件: skew 内リプレイ封じ）

目標: x0081 HTTP PoP に `x-alchemy-pop-nonce` を追加し、Room Token `jti` と組の `{jti, nonce}` を ETS 一回限りで消費する（`exp = pop_ts + skew`）。scene append / seal 等の skew 内リプレイを閉じ、P2-2 を完全クローズする。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) **P2-2** · [opus-specific-proposals-2026-11-12.md](../../0_doc/evaluation/opus/opus-specific-proposals-2026-11-12.md) · [x0081](x0081-control-plane-pop.md)（nonce は明示スコープ外）· [x00d2](x00d2-catalog-rl-epoch-api.md)（PoP nonce は別スライス）

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| PoP 必須経路で `x-alchemy-pop-nonce` 必須（空不可・最大 128 **bytes**） | PlaceRead / tokens / epoch probe 等の非 PoP 経路 |
| 署名メッセージ `alchemy/pop/v1\|METHOD\|path\|ts\|nonce\|raw_body`（protocol ↔ Elixir lockstep） | RFC 9449 DPoP JWT 形 |
| `{jti, nonce}` を ETS で一回限り；`exp = pop_ts + skew`（未来寄り ts の skew 残窓を覆う）；再利用は 401 `pop_required` | world 再起動後の ETS 揮発；downstream 失敗時の nonce 解放（host は毎回 fresh nonce） |
| 盗んだ host Room Token alone では依然 401 | session 秘密鍵 + Token 漏洩（毎リクエスト新 nonce 可） |
| PROTOCOL_PIN 非バンプ（HTTP ヘッダ契約；host+world 同時デプロイ） | 分散 world 間の共有 nonce ストア |

---

## 設計（要点）

| ヘッダ | 値 |
|:---|:---|
| `x-alchemy-pop-ts` | Unix 秒 |
| `x-alchemy-pop-nonce` | 16B 乱数の base64url（no pad）；host が毎リクエスト生成 |
| `x-alchemy-pop-sig` | base64url(Ed25519(msg)) |

`msg = alchemy/pop/v1|{METHOD}|{request_path}|{ts}|{nonce}|{raw_body}`  
検証順: skew → sig → Ed25519(`cnf.spk`) → ETS `insert_new({jti, nonce}, pop_ts+skew)`。  
`exp = pop_ts + skew` により、未来寄り `ts`（最大 `now+skew`）でも claim から skew 秒後に prune されても `|now'-ts|<=skew` の遅延リプレイを許さない。失敗は一律 401 `pop_required`（nonce は解放しない；host は毎回 `fresh_pop_nonce`）。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | protocol `http_pop` nonce 契約 | **done** |
| 2 | world HttpPop + Nonce ETS | **done** |
| 3 | host `http_pop_client` | **done** |
| 4 | リプレイ否定テスト + docs / P2-2 closed | **done** |

---

## 完了条件

1. 正規 host（Token + PoP + 新 nonce）は host_keys / scene / seal が通る。
2. 同一 `ts`/`nonce`/`sig`/body の即再送は skew 内でも 401。
3. 未来寄り `ts` 受理後、claim 相対 TTL が切れる想定時点でも同一キャプチャ再送は 401（`exp = ts+skew`）。
4. nonce 欠落は 401。
5. trust-boundary / improvement-plan P2-2 が PoP nonce 込みで closed。

## やらない

- PlaceRead / auth-JWT 経路への PoP
- 永続 nonce ストア / クラスタ共有
- PROTOCOL_PIN bump

## 関連

- [x0081-control-plane-pop.md](x0081-control-plane-pop.md)
- [x00d2-catalog-rl-epoch-api.md](x00d2-catalog-rl-epoch-api.md)
- [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md)

← [目次](../README.md)
