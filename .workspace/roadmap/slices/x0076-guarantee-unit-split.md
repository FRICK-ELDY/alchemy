# x0076 — host room_token / client main 保証単位分割

> **ID:** `x0076` · **slug:** `guarantee-unit-split` · **status:** `done` · **area:** `[host, client, docs]` · **depends_on:** `[x0075, x0042, x003d]`

目標: improvement-plan **P2-4** — host `room_token.rs` と client `main.rs` を **保証単位でファイル分割**する。挙動変更なし。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P3** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P2-4** · [opus-specific-weaknesses-2026-10-30.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-10-30.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 分割後も公開パス維持（`host_server::room_token::*`、client の join/bind/leave/`close_zenoh`） | 挙動・ワイヤ・TTL・fail-closed の変更 |
| 各ファイル先頭に Guarantees / Non-guarantees を再配分 | 新規機能・DoS 緩和（bind 送信元分割は別） |
| host: JWKS / verify / credential chain / worker / seal / scene を別ファイル | NIF・crate 抽出 |
| client: bus（manifest/frame/presence/input/liveliness）+ assets_fetch + room + 薄い main | lib.rs 化・hotswitch 再設計 |

## 設計

### host-server

`client/host-server/src/room_token.rs` → `room_token/` ディレクトリ。`lib.rs` の `pub mod room_token` は維持。

| ファイル | 保証単位 |
|:---|:---|
| `jwks.rs` | JWKS fetch / SWR / hard-stale / BG（verify 経路で HTTP しない） |
| `verify.rs` | RS256 + iss/aud/exp/room_id/role/cnf.spk（HTTP なし） |
| `timing.rs` | soft 80% / hard TTL / publish gate（純関数） |
| `http.rs` | credential HTTP agent・blake3 hex 形式 |
| `credential_chain.rs` | auth → host Token → attestation 三層 |
| `seal_cache.rs` | ManifestSeal HTTP + キャッシュ TTL |
| `room_scene.rs` | world 権威 scene GET/append |
| `credential_worker.rs` | 専用スレッド・snapshot のみ physics 公開 |

### client/app

| ファイル | 保証単位 |
|:---|:---|
| `assets_fetch.rs` | Assets GET タイムアウト・placeholder / debug fixture |
| `bus/{wire,manifest,frame,presence,input,liveliness}.rs` | AEAD 境界と各購読/配信（`bus` は `room` に依存しない） |
| `room.rs` | Zenoh open/close、join/bind/leave、token refresh、bus 起動 |
| `main.rs` | boot / SharedState / silent login (auth only) / `surface::run` |

## 本 Slice に含めない

| 項目 | 予定 |
|:---|:---|
| bind 送信元分割 | **x0077**（P2-5） |
| mTLS temp・DemoScene ベクタ・metrics+jti | 別 Slice |
| 挙動変更・検証スキップ | 禁止 |

## 完了条件

1. host `room_token/` 分割・既存 unit / fuzz パス緑
2. client `main.rs` 分割・`surface/desktop` の公開面維持
3. Slice / backlog / README 鮮度一致

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | host `room_token/` 分割 | done |
| 2 | client `main` 分割 | done |
| 3 | テスト + docs done | done |

← [目次](../README.md)
