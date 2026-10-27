# x0026 — Manifest 全体の world 署名（T3 残・D4 残核）

> **ID:** `x0026` · **slug:** `manifest-world-seal` · **status:** `done` · **area:** `[world, protocol, host, client, docs]` · **depends_on:** `[x0025]`

目標: Zenoh 上の `SceneManifest` **本体**（entries / locomotion / pin / host_pub / host_attestation）が改ざんされていないことを、world JWKS で検証できるようにする。  
**SceneManifest 駆動（P4 / D2）は入れない** — 定義の正本は引き続き host ハードコード。world は「host が載せたバイト列」に封印を与えるだけ。

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [world-server.md](../../0_doc/architecture/world-server.md) · [client.md](../../0_doc/architecture/client.md) · [peer-delivery.md](../../../protocol/peer-delivery.md)

## 設計方針

| ID | 層 | 内容 |
|:---|:---|:---|
| T3 | protocol / client | Manifest 本体の改ざんを world seal で検出。失敗は leave |
| D4 | world | host Room Token + owner のみが seal を取得できる |
| 整合 | host | seal 取得失敗・期限切れは x0025 と同じ **fail-closed**（公開停止） |
| 分離 | — | `host_attestation` は残す（鍵由来）。seal は内容完全性。畳まない |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | world `manifest_seal` + ExUnit | done |
| 2 | proto `world_seal` + PIN bump + wire helper | done |
| 3 | host: body→blake3→seal→publish；TTL キャッシュ | done |
| 4 | client: JWKS で seal 検証→blake3 一致→既存 attestation | done |
| 5 | architecture / peer-delivery | done |

## 契約（固定した判断）

### 1. 誰がいつ署名するか

- **host が body を組み立て**、封印対象バイトの BLAKE3 を計算する
- **world が JWT seal を発行**する（Room Token と同じ JWKS / RS256）
- host は seal を Manifest に載せて Zenoh publish する
- client / （任意で）host 自己検査は world JWKS で seal を検証する

world が entries を定義して配る流れにはしない（それが P4）。

### 2. `host_attestation` との関係

| フィールド | 役割 |
|:---|:---|
| `host_attestation` | `(room_id, host_pub)` の由来（x0025）。**残す** |
| `world_seal`（新規） | Manifest **内容**の封印（本 Slice） |

畳まない理由: 鍵ローテと scene 改訂の TTL / 失敗モードを分離するため。

### 3. 再送・キャッシュ

- Manifest の late-joiner 再送（〜5s）で **body と revision が不変なら seal を再利用**
- seal は TTL の 80% で再取得（x0025 credential と同じ fraction）
- body / revision が変わったら即再 seal。失敗は fail-closed
- credential HTTP timeout（x0025）を流用

### ManifestSeal JWT

| Claim | 値 |
|:---|:---|
| `typ` | `manifest_seal` |
| `room_id` | 部屋 ID |
| `revision` | Manifest.revision |
| `manifest_blake3` | 封印対象バイトの BLAKE3（hex） |
| `sub` | owner（Room Token の sub） |
| `iss` | `alchemy-world` |
| `aud` | `alchemy-manifest-seal` |
| `exp` / `jti` | 短命 / UUID（TTL = `room_token_ttl_seconds`、現状 300s。HostAttestation と同値。独立短縮の予定なし） |

発行: `POST /api/v1/rooms/:room_id/manifest_seal`  
Authorization: Bearer **host Room Token**（owner）。  
body: `{ "revision": <u64>, "manifest_blake3": "<hex>" }`  
→ `{ "seal", "expires_in", "room_id", "token_type" }`。

条件: `role=host`、path `room_id` 一致、`Rooms.owner?`。未作成 404。guest 403。発行失敗は 5xx（token 検証失敗は 401）— x0025 host_keys と同じ分離。

### 封印対象バイト（canonical）

1. `SceneManifest` を組み立てる（`world_seal` は **空文字**）
2. protobuf encode
3. そのバイト列の BLAKE3 hex が JWT の `manifest_blake3`
4. 検証後にワイヤへ載せるときは同じ body に `world_seal = seal JWT` を入れて encode（**seal フィールドはハッシュに含めない**）

`host_attestation` は封印対象に **含める**（attestation 差し替えも改ざん扱い）。

### ワイヤ

- `SceneManifest.world_seal`（string JWT）を追加
- client 適用順: `protocol_pin` → `room_id` → **world_seal**（JWKS + blake3）→ `host_attestation`（既存）→ `host_pub` 採用
- `PROTOCOL_PIN`: `alchemy-protocol/0.11.0-x0026`

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| seal なし・偽 seal・blake3 不一致 Manifest → client leave | SceneManifest 駆動・定義の world 正本化（P4 / D2） |
| guest Token では manifest_seal 不可 | Room 永続・create race の本解決 |
| revision/body 不変時は seal キャッシュで再送可能 | Presence / Frame の world 署名 |
| seal refresh 失敗は fail-closed（旧 seal の期限切れ継続公開なし） | seal 失敗後の自動復旧 |
| `host_attestation` と seal の二重検証 | attestation を seal に畳むこと |

## やらない（後続）

- SceneManifest 駆動 / shared-sim / EntityKind（P4）
- world が entries を生成して host に配る
- Friendship / ホスト移譲 / verifier
- Manifest を assets CAS の Blob にする（VCS）

## 成功条件

1. seal なし・偽 JWT・blake3 不一致 Manifest → client leave  
2. host が entries だけ改変して旧 seal のまま publish → client leave  
3. guest Token → manifest_seal 403；owner host Token → 200  
4. body 不変の republish で world を毎回叩かない（キャッシュ）  
5. seal refresh 失敗時は Manifest 公開停止  
6. `cargo test -p alchemy-protocol -p client -p host-server` と world ExUnit / hermetic E2E 緑  

## 起動メモ

PIN が上がるため host/client を揃えて再起動。日常スタックは x0025 と同様。

```bash
cd client && cargo test -p alchemy-protocol -p client -p host-server
cd world-server && mix test
```

← [目次](../README.md)
