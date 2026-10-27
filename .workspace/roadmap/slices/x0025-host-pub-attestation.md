# x0025 — host_pub 証明（D4 + T3 残核）

> **ID:** `x0025` · **slug:** `host-pub-attestation` · **status:** `done` · **area:** `[world, protocol, host, client, docs]` · **depends_on:** `[x0024]`

目標: Frame の信任根である Manifest `host_pub` を **world が証明**する。Presence も同じ attested 鍵で `host_sig` 検証する。Manifest 全体の world 署名・P4 定義駆動は入れない。

正本: [protocol.md](../../0_doc/architecture/protocol.md) · [world-server.md](../../0_doc/architecture/world-server.md) · [client.md](../../0_doc/architecture/client.md) · [peer-delivery.md](../../../protocol/peer-delivery.md)

## 設計方針

| ID | 層 | 内容 |
|:---|:---|:---|
| D4 | world | `POST …/host_keys`：Bearer **host Room Token** + owner。`(room_id, host_pub)` を HostAttestation JWT（world JWKS） |
| T3 | protocol / client | Manifest に `host_attestation`。client は JWKS 検証後のみ `host_pub` 採用 |
| T3 | protocol / host / client | Presence に `host_sig`（Frame と同方式） |
| 硬化 | host / client | 自己検査 `role=host` 必須。`ALCHEMY_ROOM_TOKEN` は debug のみ |
| 常駐 | host | auth JWT → host Room Token → HostAttestation の三段更新（各 TTL の 80%、credential HTTP timeout）。refresh 失敗は **fail-closed**（Manifest 公開停止・終了）。`ALCHEMY_AUTH_TOKEN` は長命 JWT 前提（短命静的 token は host Token 再発行時に落ちる） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | world host_keys + ExUnit | done |
| 2 | proto PIN + Presence sig | done |
| 3 | host attest / client verify / bypass | done |
| 4 | architecture / peer-delivery | done |

## 契約

### HostAttestation JWT

| Claim | 値 |
|:---|:---|
| `typ` | `host_attestation` |
| `room_id` | 部屋 ID |
| `host_pub` | Ed25519 公開鍵 32B の base64url（padding なし） |
| `sub` | owner（Room Token の sub） |
| `iss` | `alchemy-world` |
| `aud` | `alchemy-host-attestation` |
| `exp` / `jti` | 短命 / UUID |

発行: `POST /api/v1/rooms/:room_id/host_keys` body `{ "host_pub": "<base64url>" }` → `{ "attestation", "expires_in" }`。  
条件: Room Token `role=host`、path の `room_id` 一致、`Rooms.owner?`。未作成は 404。

### ワイヤ

- `SceneManifest.host_attestation`（string JWT）。`host_pub` 残置；client は attestation 内とバイト一致
- `PresenceState.host_sig`（64B）。encode 時 sig 空で Ed25519
- `PROTOCOL_PIN`: `alchemy-protocol/0.10.0-x0025`

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 偽 Manifest の自己申告 `host_pub` だけでは Frame/Presence の信任根にならない | Manifest 全体の world 署名・entries 正本（P4） |
| Presence 改ざんを attested `host_pub` で検出 | Presence の world 署名 |
| guest Token では host_keys 不可 | Room 永続・create race の本解決 |
| release で `ALCHEMY_ROOM_TOKEN` が create/tokens を迂回しない | debug での bypass 廃止 |
| host は Room Token / attestation TTL を超えて常駐可能（三段 refresh） | refresh 失敗後の自動復旧・旧 JWT の継続公開 |

## やらない（後続）

- Manifest 全体 JWS / SceneManifest 駆動（D2 / P4）
- Friendship / ホスト移譲 / verifier

## 成功条件

1. attestation なし・偽 JWT・`host_pub` 不一致 Manifest → client leave  
2. 無署名 / 偽 sig Presence → 適用しない  
3. guest Token → host_keys 403；owner host Token → 200  
4. release で `ALCHEMY_ROOM_TOKEN` bypass 無効  
5. `cargo test -p alchemy-protocol -p client -p host-server` と world ExUnit / hermetic E2E 緑  
6. host は Room Token TTL 経過後も三段 refresh で Manifest を更新し続け、refresh 失敗時は公開を止めて終了する  

## 起動メモ

日常スタックは x0024 と同様。PIN が上がるため host/client を揃えて再起動。

```bash
# world-server / auth / zenohd / host-server / client
cd client && cargo test -p alchemy-protocol -p client -p host-server
cd world-server && mix test
```

← [目次](../README.md)
