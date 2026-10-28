# x003d — host credential worker 分離（P1-3）

> **ID:** `x003d` · **slug:** `credential-worker` · **status:** `done` · **area:** `[host, docs]` · **depends_on:** `[x0025, x0026, x0039]`  
> **優先度:** **P1-3** — 物理ループから資格情報 HTTP を外す

目標: auth / Room Token / HostAttestation / ManifestSeal の更新を専用スレッドへ移し、物理 60Hz は [`HostPublishSnapshot`](../../../client/host-server/src/room_token.rs) を読むだけにする。一時的な HTTP 失敗ではプロセスを終了せず、**hard expiry（フル TTL）後だけ** Manifest / Presence / Frame / snapshot queryable の配信を止める。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P1-3** · 評価弱点「物理ループ同期 HTTP + 1 回失敗で部屋消失」

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 物理ループ経路に credential / seal の ureq が無い | Zenoh `put().wait()` を別ワーカーへ移すこと |
| soft deadline 失敗は backoff 再試行し、最後の良い snapshot を保持 | auth 永久死でも無限に配信すること |
| attest 成功後に seal が失敗しても `pending_seal` で seal 再試行（backoff 対象） | — |
| hard expiry 後は Manifest / Presence / Frame / **snapshot queryable** を出さない（プロセスは生存）。物理側も `hard_expire_at` を毎 tick 評価 | launcher 再起動ポリシーの変更 |
| 配信再開時は wire 上の `last_sent` から消えた id の `removes` を合成し、現存は full upsert で catch-up | client 側の自動 snapshot 再取得 |
| 起動時 bootstrap 失敗は従来どおり起動しない | network publish キュー / backpressure |

## 設計

1. **`HostPublishSnapshot`** — `publish_allowed` / `hard_expire_at` / sealed `manifest_bytes` / `host_generation` / `manifest_epoch`
2. **credential-refresh スレッド** — 事前 `plan` + `apply_credential_plan` + seal `ensure`；成功時のみ snapshot 更新；seal 失敗時は `force_seal_retry`；一時失敗は warn + 指数 backoff（1s→30s）
3. **物理ループ** — 毎 tick `publish_allowed_at(now, hard_expire_at, …)`；ゲート中は `last_sent`（最終 wire ビュー）を保持・更新しない；再開 net tick で removes 合成 + full upsert；manifest は一回の read で `(epoch, revision, bytes)`
4. **fail-closed** = 失効で配信停止（Frame と同列に snapshot queryable も拒否）。一時 502 で `return Err` しない

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 + P1-3 記載 | done |
| 1 | snapshot / hard expiry / worker + main ゲート | done |
| 2 | 単体テスト + architecture / improvement-plan | done |

## 手動確認（任意）

日常スタックで host 起動後、auth を一時停止しても host プロセスが即終了しないこと。TTL 内は配信継続、hard expiry 後は publish が止まり physics ログは続くこと。

## 完了条件

- `cargo test -p host-server` / clippy `-D warnings` 緑
- 物理ループに `refresh_if_due` / `ensure_sealed_manifest` の同期呼び出しが無い
- P1-3 を improvement-plan 上 done
