# x0073 — P3 衛生（軽い一括）

> **ID:** `x0073` · **slug:** `p3-hygiene-easy` · **status:** `done` · **area:** `[auth, docs, ci, launcher, assets, world]` · **depends_on:** `[x0072, x0049, x0068]`

目標: improvement-plan P2 帯のうち **小さく閉じられる衛生**を一括で閉じる。重い分割・常駐 Port・reaper/GC は含めない。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P0-次 / P3** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P2-3 / P2-7 / P2-8 / P2-10** + README 鮮度

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `Auth.Token.Keys` の signer / JWKS 読取は `:persistent_term`（GenServer 直列化なし） | 実行時キーローテーションのゼロダウンタイム（再起動で PT 更新） |
| 鍵ロードは `init/1` 同期。`start_link` 完了＝PT 投入済み。未準備読取は raise（空 JWKS を 200 で返さない） | 再起動窓の 5xx 回避（erase〜再 put は fail-closed） |
| prod compose のホスト publish 既定は loopback（`127.0.0.1:PORT`） | コンテナ内 `BIND_ALL` / Docker bridge の変更 |
| launcher も `cargo deny check`（CI + `ci.sh`） | client と同一 advisory ignore の完全同期 |
| 腐った `smoke-host-token.sh` を削除 | 全 smoke の CI 必須化 |
| README / roadmap 現状が x0072 閉じ・次 ID と一致 | vision 本文の全面書き直し |

## 本 Slice に含めない（後続）

| 項目 | 予定 |
|:---|:---|
| reserved reaper / orphan CAS GC | **x0074** |
| BLAKE3 / seal-hash 常駐 Port | **x0075** |
| host `room_token.rs` / client `main.rs` 分割 | **x0076** |
| bind 送信元分割・mTLS temp 掃除・DemoScene ベクタ・jti metrics | 別 Slice / 並行可 |

## 完了条件

1. Keys 読取が PT 経由・既存 keys / token テスト緑
2. auth/world/assets `docker-compose.prod.yml` の ports が `127.0.0.1:` 既定
3. launcher `deny.toml` + workflow / `ci.sh` deny
4. `smoke-host-token.sh` 削除
5. README・roadmap・backlog 鮮度更新

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | Keys `:persistent_term` | done |
| 2 | prod publish loopback | done |
| 3 | launcher cargo deny | done |
| 4 | smoke 削除 + docs | done |

← [目次](../README.md)
