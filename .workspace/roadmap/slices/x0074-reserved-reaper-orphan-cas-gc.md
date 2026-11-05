# x0074 — reserved reaper + orphan CAS GC（P2-1）

> **ID:** `x0074` · **slug:** `reserved-reaper-orphan-cas-gc` · **status:** `done` · **area:** `[assets, docs]` · **depends_on:** `[x003f, x0073]`

目標: クラッシュ等で残る Quota 予約を TTL 回収し、`CasObject` 行のない CAS bytes（x003f 孤児）を grace 後に安全削除する。upload 補償パスは **引き続き `CAS.delete` 禁止**。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P0-次** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P2-1** · [assets-server.md](../../0_doc/architecture/assets-server.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `quota_reservations` 行単位の予約（owner×blake3×bytes×`reserved_at`） | Oban / 署名 URL 直 PUT |
| TTL 超過予約の解放と `reserved_bytes` 減算（GenServer `send_after`） | 実行中 PUT をゼロ失敗にする超短 TTL（運用で余裕を取る） |
| CasObject 無し・in-flight 予約無し・grace 後の CAS bytes 削除 | CasObject あり・refcount=0 の削除（将来） |
| upload compensate は引き続き予約解放のみ（CAS 非削除） | S3 バケット全 listing スキャン（候補は DB `cas_orphan_candidates`） |
| 並行同一 blake3 の共有 bytes を compensate / 早すぎる GC で消さない | マイグレ前に残った blake3 不明の `reserved_bytes`（起動時に 0 クランプ） |

## 設計

1. **`quota_reservations`**: 予約 TX で insert。finalize / compensate / reaper で delete + `reserved_bytes` 減算
2. **`Assets.Quota.ReservedReaper`**: `Auth.TokenCleanup` 同型。TTL 切れ行を回収し、CasObject 無しなら orphan 候補へ
3. **`cas_orphan_candidates`**: compensate / reaper / local スキャンで `noticed_at` 記録
4. **`Assets.CAS.OrphanGc`**: grace 経過かつ CasObject・予約なし → `CAS.delete`（upload 経路とは別）

## 本 Slice に含めない

| 項目 | 予定 |
|:---|:---|
| BLAKE3 / seal-hash 常駐 Port | 別 Slice |
| host `room_token.rs` / client `main.rs` 分割 | 別 Slice |
| BlobRef 無し CasObject の refcount GC | 将来 |

## 完了条件

1. crash 相当（予約行のみ残置）で reaper が `reserved_bytes` を戻す
2. 並行 2 予約のうち TTL 切れ分だけ部分解放できる
3. finalize 失敗テストは従来どおり（compensate・CAS 非削除）
4. orphan: grace 後に bytes 削除；予約中 / CasObject ありは削除しない
5. `mix test`（assets）緑。保証表・コード・slice 一致

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | migration + 予約行配線（Blobs） | done |
| 2 | ReservedReaper GenServer | done |
| 3 | OrphanGc + candidates | done |
| 4 | tests + architecture / threat-model / README | done |

← [目次](../README.md)
