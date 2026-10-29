# x003f — CAS object/ref 分離 + put 補償（P1-2）

> **ID:** `x003f` · **slug:** `cas-object-ref-reserve` · **status:** `done` · **area:** `[assets, docs]` · **depends_on:** `[x0003, x001c, x002d]`  
> **優先度:** **P1-2** — 物理 CasObject と owner BlobRef を分離し、Quota 予約→TX 外 put→振替／補償する

目標: `cas_objects`（物理メタ）と `blob_refs`（owner×blake3）を分離する。PUT は `reserved_bytes` で短 TX 予約 → トランザクション外で CAS put → 短 TX でメタ確定・used 振替。失敗時の補償は **reserved 解放のみ**（`CAS.delete` しない＝並行同一 hash 安全）。cross-owner は実バイト再送（PoP）で自 ref + Quota 消費。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P1-2** · [assets-threat-model.md](../../0_doc/architecture/assets-threat-model.md) ② · 正本 [assets-server.md](../../0_doc/architecture/assets-server.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `cas_objects` は所有者を持たない物理メタ（blake3 PK） | BlobGrant / StorageVault（→ draft / P4-5） |
| `blob_refs` は UNIQUE(owner_sub, blake3)。cross-owner は各自 ref + Quota | hash だけの `:owned` 登録（PoP なし） |
| PUT: reserved → TX 外 put → used 振替。失敗時 reserved 解放 | Oban・予約期限切れジョブ・署名 URL 直 PUT |
| — | 同一 owner 同一 blake3 の並行 PUT で、相手 ref 確定前に枠不足 → その要求は `quota_exceeded`（再試行で `:existing`） |
| 並行同一 blake3（cross-owner PoP 含む）で共有 CAS を消さない | orphan CAS（CasObject 無し）の定期 GC |
| メタ確定失敗の補償は **予約解放のみ**（CAS.delete しない） | refcount / 「未参照なら削除」ジョブ |
| GET は CasObject 存在 + Bearer（owner 照合なし＝現行 capability） | 非公開・有料の confidentiality |
| Quota API に `reserved_bytes` を露出 | 課金連動 Quota |

## 設計

1. **CasObject**: blake3 / size / mime（所有者なし）
2. **BlobRef**: owner_sub + blake3 → CasObject（Quota 単位）
3. **QuotaLedger**: `used_bytes + reserved_bytes <= limit_bytes` を条件付き UPDATE
4. **PUT**: TX1 予約（または自 ref で `:existing`）→ CAS.put_if_absent（外）→ TX2 upsert object + insert ref + reserved→used
5. **補償**: 予約解放のみ。`CAS.delete` しない（`:created` 誤認＋並行 put で共有実体削除を防ぐ）
6. **mime**: CasObject は先勝ち。後続 owner の Content-Type は物理メタを上書きしない

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 + P1-2 記載 | done |
| 1 | Migration + schema（cas_objects / blob_refs / reserved） | done |
| 2 | Blobs.upload（reserve → TX 外 put → finalize）。補償は予約解放のみ | done |
| 3 | cross-owner / 補償 / reserve テスト | done |
| 4 | architecture / threat-model / improvement-plan | done |

## 完了条件

- cross-owner 同 bytes → 双方 `created`・双方 quota 加算・CAS は 1 オブジェクト
- finalize 失敗後に used/reserved/refs/objects が整合（補償テスト）
- `CAS.put` が DB トランザクション外
- `mix test`（assets）緑。保証表・コード・slice 一致
- P1-2 を improvement-plan 上 done

← [目次](../README.md)
