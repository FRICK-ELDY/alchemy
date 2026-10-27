# protocol（alchemy-protocol）

> **正本**: [`protocol/`](../../../protocol/)  
> **PROTOCOL_PIN**: `alchemy-protocol/0.9.0-x0022`

## 置き場の方針

スーパープロジェクト直下の **独立クレート**。client submodule 化後もここ（または将来の `alchemy-protocol` リポ）に置き、client / host-server / Elixir サーバが参照する。

client workspace からは `path = "../protocol"` で依存する（`client` の中には埋め込まない）。

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| `SceneManifest` / `RenderFrame`（delta）/ `InputEvent` / `PresenceState` の encode / decode | 本格ビット圧縮・予測符号化 |
| `PROTOCOL_PIN` による世代識別 | フル DrawCommand 体系 |
| f64 空間座標（描画境界） | audio / tracking |
| `InputEvent.room_token` フィールドの存在 | Token の署名検証（host-server / world-server の責務） |
| Manifest / frame / presence / host_control のキー式ヘルパ + `validate_room_id`（x0017） | world-server 署名付き Manifest・テナント名前空間 |
| `LocomotionConfig`（World 定義の active mode） | モジュール一覧 UI / 抑制ソース |
| `DEFAULT_MOVE_SPEED`（host / Settings 表示の共有既定） | client 永続設定・仮想キーボード編集 |
| `PresenceState`（`sub`→user_root entity）+ `InputEvent.leave` + liveliness キーヘルパ（x0019） | 5 人以上の動的スケール / WAN liveliness ACL |

## メッセージ

### SceneManifest（`proto/scene_manifest.proto`）— 低頻度 + x0009 loco

- `SceneManifest { room_id, revision, entries[], locomotion }`
- `SceneEntry { entity_id, blake3 (hex), x,y,z }`
- `LocomotionConfig { mode, move_speed }` — `UNSPECIFIED` → host は **Noclip**
- `LocomotionMode`: NOCLIP / FLY / WALK
- **GLB バイトは載せない**（assets-server CAS 参照のみ）

### RenderFrame（`proto/render_frame.proto`）— Slice C-C・高頻度デルタ

- `RenderFrame { tick, host_time_ms, upserts[], removes[] }`
- `Entity { id, x,y,z (double), r,g,b,a (float) }`
- 静止 Entity は `upserts` に載せない（フルスナップショット廃止）
- entity: Coin=`1`、static=`2`、**user_root=`3..6`**（x0015・最大4・Token `sub` 割当）、**grabbable boxes=`10..18`**（x0010・手続き立方体、Manifest blake3 なし）

### InputEvent（`proto/input_events.proto`）— Intent（x0009）+ Grab（x0011 / x0013）+ Leave（x0019）

- `InputEvent { room_token, client_time_ms, move_x, move_y, move_z, buttons, grab_entity_id, aim_x, aim_y, aim_z, leave }`
- `move_*` はワールド投影済みの正規化 Intent（~-1..1）。Desktop=マウスルック yaw 相対（x0012）、VR=コントローラー aim 相対。host が `move_speed` でスケール
- `buttons` ビット: Primary / Secondary / Grab / Context / Jump（未使用は host 無視可）
- `grab_entity_id` + `aim_*`（ワールド aim 原点: XR コントローラ / Desktop 眼）: GRAB 中の kinematic 追従。`0` / GRAB 無しは無視
- `leave=true`: 明示退室。host は Token verify 後に即 `reclaim_sub`（移動/Grab 適用なし）
- `room_token` は world-server 発行の短命 JWT（検証は host-server）
- `DEFAULT_MOVE_SPEED = 4.0`（Rust 定数。Manifest `move_speed` が 0 のときの host 既定 / Settings 表示）

### PresenceState（`proto/presence.proto`）— x0015・低頻度（解放は x0016 / x0019）

- `PresenceState { revision, assignments[] }`
- `PresenceAssignment { sub, entity_id }` — occupied のみ。host が first-come 割当を公開
- client は自 Room Token の `sub`（payload 読み取りのみ）で `entity_id` を解決
- host 解放: **Leave** / **liveliness Delete（`sub`+`jti` 一致時のみ・x0022）** + Intent 無音 TTL 3s（フォールバック・x0016）

## キー式ヘルパ（x0017: `ROOM_ID` 本配線 + x0022 liveliness）

式: `alchemy/dev/room/{room_id}/{manifest|frame|input|presence|host_control|alive/{sub}/{jti}}`。  
`*_KEY_LOCAL` は `room_id=local` の別名。host/client は env `ROOM_ID`（既定 `local`）を `validate_room_id` してから `*_key` で declare する。

| 定数 / 関数 | 既定 |
|:---|:---|
| `MANIFEST_KEY_LOCAL` / `manifest_key(room_id)` | `alchemy/dev/room/local/manifest` |
| `FRAME_KEY_LOCAL` / `frame_key(room_id)` | `alchemy/dev/room/local/frame` |
| `INPUT_KEY_LOCAL` / `input_key(room_id)` | `alchemy/dev/room/local/input` |
| `PRESENCE_KEY_LOCAL` / `presence_key(room_id)` | `alchemy/dev/room/local/presence` |
| `HOST_CONTROL_KEY_LOCAL` / `host_control_key(room_id)` | `alchemy/dev/room/local/host_control` |
| `liveliness_key(room_id, sub, jti)` / `liveliness_wildcard(room_id)` | `…/alive/{sub}/{jti}` / `…/alive/**` |
| `liveliness_binding_from_key` | `(sub, jti)`。単セグメント旧キーは拒否 |
| `validate_room_id` | 非空・`/` なし・ASCII 印刷可能 |
