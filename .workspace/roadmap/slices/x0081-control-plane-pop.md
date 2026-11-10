# x0081 — control plane PoP（session 署名 + world HTTP PoP）

> **ID:** `x0081` · **slug:** `control-plane-pop` · **status:** `done` · **area:** `[protocol, world, host, client, docs]` · **depends_on:** `[x003e, x006d, x007b, x007e, x0080]`  
> **優先度:** 評価 P0-1（制御面 bearer Token 排除 + host API 所持証明）

目標: inventory spawn/despawn から **Room Token を Zenoh に載せない**（Intent と同型の session Ed25519）。world の host HTTP（`host_keys` / `manifest_seal` / `scene*`）は Bearer に加え **`cnf.spk` PoP** を必須にし、盗んだ host Token  alone では 401 とする。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P0-1** · [specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/specific-weaknesses-2026-11-05.md) · x007e / x0080 非保証「制御面 PoP は別 Slice」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Zenoh `inventory/spawn\|despawn` 平文に `room_token` が無い。`session_pub` + `control_seq` + `control_sig`（Ed25519） | Intent 本体の protobuf 変更；Frame / Presence の Token 排除の再設計 |
| host は SessionTable に bind 済みの session のみ受理。`sub` は **現計算ホスト**（host Token `sub`）と一致必須 | 任意 guest の take-out（非ホスト端末からの遠隔 spawn）；Invite ACL |
| world scene append/delete は **host 自前 Token + PoP**（クライアント JWT を転用しない） | Scene revision CAS（P0-3・別 Slice） |
| `host_keys` / `manifest_seal` / `GET|POST|DELETE …/scene*` は `cnf.spk` に対する PoP（`alchemy/pop/v1\|method\|path\|body_sha256\|ts`）必須。欠落・不一致・skew 超過は **401** | PlaceRead / tokens / handover 等の auth-JWT 経路への PoP；RFC 9449 完全互換 |
| 盗んだ host Room Token のみで `host_keys` を叩いても 401（秘密鍵無し） | 端末から session 秘密鍵まで盗まれた場合の防御 |
| PROTOCOL_PIN bump（制御面 JSON 契約変更） | relay-agent `key_pem` 退役（P0-4） |

---

## 設計（要点）

### 制御面（Zenoh JSON）

```
{ "session_pub", "control_seq", "control_sig", "blake3"|…pose… }  // spawn
{ "session_pub", "control_seq", "control_sig", "entity_id" }      // despawn
```

- 署名メッセージ: `alchemy/control/v1|spawn|…` / `|despawn|…`（f64 は `to_bits` hex）
- host: AEAD open → Ed25519 verify → SessionTable（bind 済み + seq 前進 + `sub == host_sub`）→ world HTTP（自 Token+PoP）

### world HTTP PoP

| ヘッダ | 値 |
|:---|:---|
| `x-alchemy-pop-ts` | Unix 秒 |
| `x-alchemy-pop-sig` | base64url(Ed25519(msg)) |

`msg = alchemy/pop/v1|{METHOD}|{request_path}|{sha256_hex(raw_body)}|{ts}`  
skew 既定 ±60s。`cnf.spk` は Token 発行時の session_pub（host CredentialChain が鍵を保持）。

### client

- take-out / despawn は **Intent session** で署名（`host_room_token` は UI「主催中」ゲートのみ・バスに載せない）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 / backlog | **done** |
| 1 | protocol: control + http_pop ヘルパ + pin bump | **done** |
| 2 | world: raw body + HttpPop + host 経路 401 否定系 | **done** |
| 3 | host: CredentialChain 鍵保持 + HTTP PoP；spawn/despawn session 認可 | **done** |
| 4 | client: inventory 署名；docs / 保証表 | **done** |

---

## 完了条件

1. spawn/despawn 平文に JWT が無い（単体または AEAD 往復テスト）。
2. host Token のみ（PoP 無し）の `host_keys` → 401。
3. 正規 host（Token + PoP）は attestation / scene / seal が通る。
4. bind 済み session + 一致 sub の spawn は world append まで進む；未 bind / 他 sub は reject。
5. architecture 保証表が「HTTP は role のみ」から PoP 必須へ更新されている。

## やらない（本 Slice）

- Scene revision CAS（P0-3）
- relay-agent CSR-only（P0-4）
- host-token 操作別 rate limit（P1）
- 任意 guest take-out の本格 ACL

## 関連

- [x003e-room-token-cnf.md](x003e-room-token-cnf.md)
- [x006d-host-take-out-control.md](x006d-host-take-out-control.md)
- [x007b-scene-despawn.md](x007b-scene-despawn.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
