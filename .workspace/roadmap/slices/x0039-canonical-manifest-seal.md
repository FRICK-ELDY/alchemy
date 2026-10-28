# x0039 — canonical ManifestSeal（署名 oracle 廃止）

> **ID:** `x0039` · **slug:** `canonical-manifest-seal` · **status:** `done` · **area:** `[world, host, protocol, docs]` · **depends_on:** `[x0026, x0027, x0028, x0038]`

目標: ManifestSeal を「host が送った任意 hash への署名」から、「world DB scene 由来の canonical body / hash への封印」に変える。owner host が任意 entity 配置を正規 seal 付きで配信できない。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P0-2**

正本: [world-server.md](../../0_doc/architecture/world-server.md) · [protocol.md](../../0_doc/architecture/protocol.md)

## 設計

| 層 | 内容 |
|:---|:---|
| protocol | `manifest-seal-hash` CLI（scene JSON → protobuf blake3）。peer と同じ `manifest_seal_blake3_hex` |
| world | DB scene + 検証済み `host_attestation` から canonical hash。host 提出 hash / revision は **照合必須** |
| host | seal 要求に `host_attestation` を同梱。ローカル hash が不一致なら fail-closed。`ALCHEMY_BLOB_BLAKE3` は拒否 |
| client | 検証順は従来どおり（seal → blake3 → attestation） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | world: scene→canonical body/hash + ExUnit | done |
| 2 | API: host hash 照合 | done |
| 3 | host: seal 取得経路の追従 | done |
| 4 | architecture / 保証表更新 | done |

## 契約

1. **正本は world DB scene**（x0027 / x0028）
2. **host は実行・配信**。定義の改変を world に黙って通さない
3. `host_attestation` は残す（鍵由来）。seal は内容完全性
4. PIN はワイヤ schema 不変のため据え置き（world `protocol_pin` 設定と Rust `PROTOCOL_PIN` を同期）

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| owner host が任意 hash で正規 `world_seal` を取れない | revision / generation replay 拒否（→ x003a） |
| seal 対象 body が DB scene + attested host key と一致 | UGC 編集 ACL・複数シーン・VCS Blob |
| guest Token では seal 不可（x0026 継続） | verifier Spot-Check |

## 付帯（CI）

- `runtime.exs`: test では `WORLD_DATABASE_PATH` を無視（P0-6 の最小。x003b の残りは prod S2S / decode limit）

## やらない

- generation / revision 単調ゲート（→ x003a）
- BlobGrant / market
- host handoff の完全実装
- world が body バイトを HTTP で返す（hash 照合で十分）
- decode limit / prod compose S2S（→ x003b）

## 成功条件

1. host が DB と異なる `manifest_blake3` を送っても seal が発行されない（422）  
2. 正当 scene 由来 body では seal 200・client 検証成功  
3. 保証表（world / protocol）とコードが一致  
4. world ExUnit + `cargo test -p alchemy-protocol -p host-server`（必要範囲）緑  

← [目次](../README.md)
