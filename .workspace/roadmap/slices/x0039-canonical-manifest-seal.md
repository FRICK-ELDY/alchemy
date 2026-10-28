# x0039 — canonical ManifestSeal（署名 oracle 廃止）

> **ID:** `x0039` · **slug:** `canonical-manifest-seal` · **status:** `planned` · **area:** `[world, host, protocol, docs]` · **depends_on:** `[x0026, x0027, x0028, x0038]`

目標: ManifestSeal を「host が送った任意 hash への署名」から、「world DB scene 由来の canonical body / hash への封印」に変える。owner host が任意 entity 配置を正規 seal 付きで配信できない。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P0-2**

正本: [world-server.md](../../0_doc/architecture/world-server.md) · [protocol.md](../../0_doc/architecture/protocol.md)

## 設計

| 層 | 内容 |
|:---|:---|
| world | DB scene（entries / loco / pin 等）+ attested `host_pub` から **deterministic** Manifest body を構築し BLAKE3 → seal |
| API | host 指定 `manifest_blake3` は **照合必須**（不一致 4xx）または **廃止**して world が body/hash/seal を返す |
| host | 自前 hash で seal を取らず、world が認めた body（または照合済み hash）だけを publish |
| client | 検証順は従来どおり（seal → blake3 → attestation）。契約の意味が変わるだけ |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | planned |
| 1 | world: scene→canonical body/hash + ExUnit | planned |
| 2 | API: host hash 照合 or world-authored 返却 | planned |
| 3 | host: seal 取得経路の追従 | planned |
| 4 | architecture / 保証表更新 | planned |

## 契約（固定したい判断）

1. **正本は world DB scene**（x0027 / x0028）。host ハードコード定義への seal はしない  
2. **host は実行・配信**。定義の改変を world に黙って通さない  
3. `host_attestation` は残す（鍵由来）。seal は内容完全性（x0026 の分離を維持）  
4. PIN bump が必要なら本 Slice で上げ、host/client を揃える  

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| owner host が任意 hash で正規 `world_seal` を取れない | revision / generation replay 拒否（→ x003a） |
| seal 対象 body が DB scene + attested host key と一致 | UGC 編集 ACL・複数シーン・VCS Blob |
| guest Token では seal 不可（x0026 継続） | verifier Spot-Check（P0-2/3 完了後） |

## やらない

- generation / revision 単調ゲート（→ x003a）
- leave→discard（→ x0038・先行）
- BlobGrant / market
- host handoff の完全実装

## 成功条件

1. host が DB と異なる `manifest_blake3` を送っても seal が発行されない（または 4xx）  
2. 正当 scene 由来 body では seal 200・client 検証成功  
3. 保証表（world / protocol）とコードが一致  
4. world ExUnit + `cargo test -p alchemy-protocol -p host-server`（必要範囲）緑  

← [目次](../README.md)
