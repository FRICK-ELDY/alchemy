# x003e — Room Token `cnf`（session_pub）（P1-4）

> **ID:** `x003e` · **slug:** `room-token-cnf` · **status:** `done` · **area:** `[world, host, client, protocol, docs]` · **depends_on:** `[x0023, x0021]`  
> **優先度:** **P1-4** — Intent bind 用 Room Token に session 公開鍵を束縛する

目標: client が Token 要求時に Ed25519 `session_pub` を送り、world が JWT claim `cnf.spk`（base64url・32B）に署名して埋め込む。host は bind 時に Intent の `session_pub` と `cnf.spk` の一致を必須にする。盗聴した JWT だけでは攻撃者の別鍵で bind できない。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P1-4** · 評価弱点「bind 時 JWT 盗聴でセッション乗っ取り（cnf 無し・後勝ち）」

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `POST …/tokens` は `session_pub`（base64url 32B）必須。欠落・不正は 422 | `jkt`（JWK thumbprint）互換 |
| Room Token に `cnf.spk` を含める（欠落 Token は host bind 拒否） | cnf なし旧 Token の過渡受理 |
| bind 時 `cnf.spk` ↔ Intent `session_pub` バイト一致 | 端末上の session 秘密鍵窃取 |
| 盗聴 JWT + 別 Ed25519 では bind 不可 | Zenoh ACL / mTLS |
| host generation 交代は新鍵を pending に保持し、Token 到着後に鍵+Token を一括 swap（取得中は旧鍵+旧 Token）。stale refresh は discard | sync fetch 失敗時は旧鍵維持＋ refresh 再試行；Manifest スレッド上の HTTP は timeout 付き |
| — | 同一鍵での後勝ち rebind（正当 refresh / reclaim） |
| HTTP credential（scene / attest / seal）は従来どおり `role` のみ（cnf バイト非使用；verify は形式検査） | host Token の cnf を Intent 以外で使うこと |
| — | debug `ALCHEMY_ROOM_TOKEN` bypass の cnf 整合 |

## 設計

1. **Claim**: `cnf: {"spk": "<base64url Ed25519 pub>"}`（RFC 7800 コンテナ + Alchemy 固有 `spk`）
2. **world**: decode → `Token.generate(..., session_pub_bytes)` → claim 埋め込み
3. **client**: `IntentSession` を Token 取得より先に用意し、その pub を POST。refresh は同鍵。generation 交代は新鍵→新 Token
4. **host bind**: JWT verify 後に `cnf.spk` decode + `session_pub` 一致。fail-closed
5. **host credential Token**: 発行 API 共通のため ephemeral pub を載せる（HTTP 門番は cnf を見ない）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 + P1-4 記載 | done |
| 1 | world `cnf.spk` + controller 422 | done |
| 2 | host bind 照合 + 否定系テスト | done |
| 3 | client / host Token 要求順 + e2e/smoke | done |
| 4 | architecture / improvement-plan | done |

## 手動確認（任意）

日常スタックで join → Intent が通り、故意に別鍵の Intent に盗んだ Token を載せても host が受理しないこと（単体テストで固定済みなら省略可）。

## 完了条件

- 有効 Room Token + 攻撃者の別 `session_pub` では host bind が通らない（単体テスト）
- 正規 client（同じ鍵で発行した Token）の bind / refresh rebind は通る
- `mix test`（world）・`cargo test -p host-server` 緑
- 保証表・コード・slice が一致。cnf なし fail-open なし
- P1-4 を improvement-plan 上 done

← [目次](../README.md)
