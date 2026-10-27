# x002e — P3 境界・契約 + ランタイム堅牢性

> **ID:** `x002e` · **slug:** `p3-boundary-runtime` · **status:** `done` · **area:** `[auth, world, protocol, host, client, docs]` · **depends_on:** `[x002d]`

目標: 評価書 P3 の **境界・契約（B）** と **ランタイム堅牢性（D）** を縦に潰す。

根拠: [improvement-plan-2026-10-26.md](../../0_doc/evaluation/improvement-plan-2026-10-26.md) P3-5 / P3-8 / P3-9 / P3-10

## 含む / 含まない

| 含む | 含まない（後続） |
|:---|:---|
| P3-5 ClientIp + CF-Connecting-IP + edge README | P3-4 `alchemy_auth_client` |
| P3-10 room_id validator を protocol 正本に統一 | P3-14 undocumented_unsafe deny |
| P3-8 Mutex poison は fatal（空 Presence 継続禁止） | shared-sim / EntityKind |
| P3-9 UI HTTP をワーカーへ（desktop click 非ブロック） | |

## 設計方針

| ID | 層 | 内容 |
|:---|:---|:---|
| P3-5 | auth / edge | 信頼プロキシ時に `CF-Connecting-IP` 優先、なければ `X-Forwarded-For`。edge README に `TRUSTED_PROXIES` 必須手順 |
| P3-10 | protocol / world | `validate_room_id`: 非空・`/` なし・≤64・`[A-Za-z0-9_-]+`。Elixir は同一規則。共有テストベクタ |
| P3-8 | host | `presence` / `sessions` / `snapshot` の Mutex poison は process fatal（空 Presence 公開しない） |
| P3-9 | client desktop | login / register / logout HTTP をワーカースレッドへ。UI スレッドは結果を poll |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Tunnel 背後で `TRUSTED_PROXIES` 設定時、レート制限 IP が CF クライアント IP になる | Cloudflare Access / 全プロキシトポロジの自動検出 |
| host/client/world の room_id 受理集合が一致 | Unicode room_id・64 超の部屋名 |
| poison 後に空 Presence を出し続けない | poison からの自動復旧 |
| desktop Home の login/register クリックが ureq 完了までイベントループを止めない | XR 上のアカウント UI（Desktop 専用のまま） |
| charset 外の既存 room_id は無い前提（`local` / hex random のみ） | Unicode / 旧自由文字列 room_id のマイグレーション |

## PIN

`alchemy-protocol/0.15.0-x002e`（room_id 規則の強化）

## 成功条件

1. ClientIp が CF-Connecting-IP を読むテスト + edge README に TRUSTED_PROXIES  
2. protocol / world の room_id ベクタが一致  
3. host: poison 時に空 Presence 継続経路が無い  
4. desktop: Auth HTTP がワーカー経由  

← [目次](../README.md)
