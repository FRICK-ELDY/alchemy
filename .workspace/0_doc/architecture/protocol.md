# protocol（alchemy-protocol）

> **正本**: [`protocol/`](../../../protocol/)  
> **配送契約**: [`protocol/peer-delivery.md`](../../../protocol/peer-delivery.md)（x0024）  
> **PROTOCOL_PIN**: `alchemy-protocol/0.11.0-x0026`

## 置き場の方針

スーパープロジェクト直下の **独立クレート**。client submodule 化後もここ（または将来の `alchemy-protocol` リポ）に置き、client / host-server / Elixir サーバが参照する。

client workspace からは `path = "../protocol"` で依存する（`client` の中には埋め込まない）。

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| `SceneManifest` / `RenderFrame`（delta）/ `InputEvent` / `PresenceState` の encode / decode | 本格ビット圧縮・予測符号化 |
| `PROTOCOL_PIN` を Manifest / Presence に載せ、不一致検出可能（x0023） | フル DrawCommand 体系・多世代同時運用 |
| f64 空間座標（描画境界） | audio / tracking |
| Intent: Ed25519 `session_pub` / `intent_seq` / `intent_sig` + 任意 `room_token`（bind 時）（x0023） | Token JWKS 検証そのもの（host-server） |
| Frame / Presence: `host_sig` + Manifest `host_pub`（HostAttestation）+ Manifest **`world_seal`**（x0026） | SceneManifest 駆動（P4）・テナント名前空間 |
| 配送クラス表（[`peer-delivery.md`](../../../protocol/peer-delivery.md)）+ `decide_frame_tick` / `FRAME_TICK_GAP_RESYNC`（x0024） | 確実配送 ACK / event ID チャネル・欠落デルタの再送 |
| Manifest / frame / presence / host_control / snapshot のキー式ヘルパ + `validate_room_id`（x0017） | Zenoh ACL / mTLS |
| `LocomotionConfig`（World 定義の active mode） | モジュール一覧 UI / 抑制ソース |
| `DEFAULT_MOVE_SPEED`（host / Settings 表示の共有既定） | client 永続設定・仮想キーボード編集 |
| `PresenceState`（`sub`→user_root entity）+ `InputEvent.leave` + liveliness キーヘルパ（x0019 / x0022） | 5 人以上の動的スケール / WAN liveliness ACL |

## メッセージ

### SceneManifest（`proto/scene_manifest.proto`）— 低頻度 + x0009 loco + x0023 / x0025

- `SceneManifest { room_id, revision, entries[], locomotion, protocol_pin, host_pub, host_attestation, world_seal }`
- `SceneEntry { entity_id, blake3 (hex), x,y,z }`
- `LocomotionConfig { mode, move_speed }` — `UNSPECIFIED` → host は **Noclip**
- `LocomotionMode`: NOCLIP / FLY / WALK
- `protocol_pin` / `host_pub`（32B Ed25519）+ `host_attestation` + **`world_seal`**（world JWT・body blake3）: client は pin / seal / attestation 失敗で leave；Frame/Presence は attested `host_pub`（x0026）
- **GLB バイトは載せない**（assets-server CAS 参照のみ）

### RenderFrame（`proto/render_frame.proto`）— 高頻度デルタ + 署名（x0023）

- `RenderFrame { tick, host_time_ms, upserts[], removes[], host_sig }`
- `Entity { id, x,y,z (double), r,g,b,a (float) }`
- 静止 Entity は `upserts` に載せない。late join / **tick 欠落再同期**は `…/snapshot` queryable（同スキーマ・全 Entity upsert）
- Snapshot 成功条件: **`upserts` 非空**（空は未準備。host は `reply_err`）。client は snapshot に tick ゲートをかけない
- デルタ tick: host は空デルタで `net_tick` を進めない。snapshot キャッシュ tick は **最後に publish した値**（次の未送 `net_tick` 禁止）。client は `FRAME_TICK_GAP_RESYNC`（既定 1）超過で適用せず snapshot（x0024）
- `host_sig`: encode（sig 空）を host 秘密鍵で署名
- entity: Coin=`1`、static=`2`、**user_root=`3..6`**（x0015）、**grabbable boxes=`10..18`**（x0010）

### InputEvent（`proto/input_events.proto`）— Intent + session（x0023）

- `InputEvent { room_token, client_time_ms, move_*, buttons, grab_*, aim_*, leave, session_pub, intent_seq, intent_sig }`
- **Bind**: 有効 JWT + `session_pub`(32) + 単調 `intent_seq` + `intent_sig`(64)。host は **同一 `sub` の他 pub を破棄**して pubkey→`(sub,jti)` を記録（後勝ち・1 session/sub）
- **Steady**: `room_token` 空。同じ `session_pub` + 進んだ `seq` + 有効 sig のみ受理（バスで拾った JWT だけではなりすませない）
- **再 bind**: host 世代交代 → client は鍵ごと作り直し；Presence reclaim → 同じ鍵で Token 付き Intent を再開
- `leave=true`: セッション検証後に即 `reclaim_sub`
- `move_*` は正規化 Intent（~-1..1）。host が `move_speed` でスケール
- `DEFAULT_MOVE_SPEED = 4.0`

### PresenceState（`proto/presence.proto`）— x0015・低頻度（解放は x0016 / x0019 / x0022 / x0025）

- `PresenceState { revision, assignments[], protocol_pin, host_sig }`
- `PresenceAssignment { sub, entity_id }` — occupied のみ
- client は pin 不一致・`host_sig` 失敗を拒否；自 `sub` で `entity_id` を解決（x0025）
- host 解放: **Leave** / **liveliness Delete（`sub`+`jti`）** + Intent TTL 3s

## キー式ヘルパ（x0017 + x0022 liveliness + x0023 snapshot）

式: `alchemy/dev/room/{room_id}/{manifest|frame|input|presence|host_control|snapshot|alive/{sub}/{jti}}`。  
`*_KEY_LOCAL` は `room_id=local` の別名。host/client は env `ROOM_ID`（既定 `local`）を `validate_room_id` してから `*_key` で declare する。

| 定数 / 関数 | 既定 |
|:---|:---|
| `MANIFEST_KEY_LOCAL` / `manifest_key(room_id)` | `alchemy/dev/room/local/manifest` |
| `FRAME_KEY_LOCAL` / `frame_key(room_id)` | `alchemy/dev/room/local/frame` |
| `INPUT_KEY_LOCAL` / `input_key(room_id)` | `alchemy/dev/room/local/input` |
| `PRESENCE_KEY_LOCAL` / `presence_key(room_id)` | `alchemy/dev/room/local/presence` |
| `SNAPSHOT_KEY_LOCAL` / `snapshot_key(room_id)` | `alchemy/dev/room/local/snapshot` |
| `HOST_CONTROL_KEY_LOCAL` / `host_control_key(room_id)` | `alchemy/dev/room/local/host_control` |
| `liveliness_key(room_id, sub, jti)` / `liveliness_wildcard(room_id)` | `…/alive/{sub}/{jti}` / `…/alive/**` |
| `liveliness_binding_from_key` | `(sub, jti)`。単セグメント旧キーは拒否 |
| `validate_room_id` | 非空・`/` なし・ASCII 印刷可能 |
| `wire_crypto` | Intent / Frame の Ed25519 sign・verify |
