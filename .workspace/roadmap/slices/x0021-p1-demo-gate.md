# x0021 — P1 デモゲート

> **ID:** `x0021` · **slug:** `p1-demo-gate` · **status:** `done` · **area:** `[client, host, world, auth]` · **depends_on:** `[x0020]`

目標: 2026-10-26 評価の **P1**（デモ信頼性・入力ゲート）を塞ぎ、入室後 10 分超でも操作でき、改ざん入力・偽 host 役割で壊れないことを保証する。

根拠: [improvement-plan-2026-10-26.md](../../0_doc/evaluation/improvement-plan-2026-10-26.md)

## 設計方針

| ID | 内容 |
|:---|:---|
| P1-1 | Room Token / access を TTL の ~80% で再取得。`expires_in` を保持 |
| P1-2 | host JWKS はバックグラウンド更新 + stale-while-revalidate。入力スレッドから同期 HTTP 除去。ureq タイムアウト・未知 kid クールダウン |
| P1-3 | host で move を単位球内に clamp。Grab 開始は aim↔entity 距離ゲート（client pick と同スケール） |
| P1-4 | Room に `owner_sub`（**`POST /rooms` のみ**）。Token 発行は create しない。`role=host` は owner のみ |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | P1-4 world owner_sub + host 発行制限 | done |
| 2 | P1-3 host magnitude / Grab 距離 | done |
| 3 | P1-2 JWKS bg + SWR | done |
| 4 | P1-1 client Token / access refresh | done |
| 5 | 成功条件確認・後続リンク | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 入室 10 分以上、Intent が Token 期限で沈黙しない（自動 refresh） | session MAC / DPoP（P2-2）・毎フレーム Token 廃止 |
| world 一時ダウン中もキャッシュ鍵で入力検証を継続（SWR） | Elixir JWKS 共通化（P3-4） |
| `1e30` move や遠距離 Grab 開始が物理を発散させない | クライアント側のみの改ざん防止 |
| 非 owner の `role=host` 要求が 403。未作成 room への Token は 404（guest が owner にならない） | Friendship / 永続 Room DB / 公開範囲 |

## やらない（後続）

- SceneManifest 駆動（P4 / 次 Slice）
- P2 liveliness 紐付け / WAN TLS / client Frame 検証
- Quota / Plug.Parsers / `cargo deny`（P3）
- GUI E2E・Token refresh の Compose 長時間 assert

## 成功条件

1. client が Room Token TTL×0.8 で再取得し、access も refresh 可能なら更新する  
2. `RoomTokenVerifier::verify` が入力経路で同期 HTTP しない（単体 or 設計コメント + bg 起動）  
3. host 単体テスト: magnitude>1 を正規化、Grab 距離超過で開始拒否  
4. world テスト: 未作成 room への Token → 404；owner 以外の host → 403；owner の host / 既存 room の guest → 200

## 保証しない（明示）

- **create 先取り**: 任意の認証ユーザーが先に `POST /rooms`（例: 既定 `local`）すると owner になり、後続 host は 409→403。Token 経路での昇格は塞いだ。運用は host-server を先に起動する

## 起動メモ

```bash
# 日常スタック + Desktop 手通し（x0020）のまま
# **host-server を先に起動**（POST /rooms で owner になる）。client / e2e-probe は create しない。
# 入室後 5〜6 分（TTL 300s × 0.8）で client ログに Room Token refresh が出ること

cd world-server && mix test
cd client && cargo test -p host-server -p client
```

← [目次](../README.md)
