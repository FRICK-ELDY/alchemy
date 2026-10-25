# protocol（alchemy-protocol）

> **正本**: [`protocol/`](../../../protocol/)  
> **PROTOCOL_PIN**: `alchemy-protocol/0.5.0-x0011`

## 置き場の方針

スーパープロジェクト直下の **独立クレート**。client submodule 化後もここ（または将来の `alchemy-protocol` リポ）に置き、client / host-server / Elixir サーバが参照する。

client workspace からは `path = "../protocol"` で依存する（`client` の中には埋め込まない）。

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| `SceneManifest` / `RenderFrame`（delta）/ `InputEvent` の encode / decode | 本格ビット圧縮・予測符号化 |
| `PROTOCOL_PIN` による世代識別 | フル DrawCommand 体系 |
| f64 空間座標（描画境界） | audio / tracking |
| `InputEvent.room_token` フィールドの存在 | Token の署名検証（host-server / world-server の責務） |
| Manifest と frame のキー式ヘルパ | world-server 署名付き Manifest |
| `LocomotionConfig`（World 定義の active mode） | モジュール一覧 UI / 抑制ソース |
| `DEFAULT_MOVE_SPEED`（host / Settings 表示の共有既定） | client 永続設定・仮想キーボード編集 |

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
- entity: Coin=`1`、static=`2`、**user_root=`3` / `4`**（x0014・最大2・Token `sub` 割当）、**grabbable boxes=`10..18`**（x0010・手続き立方体、Manifest blake3 なし）

### InputEvent（`proto/input_events.proto`）— Intent（x0009）+ Grab（x0011 / x0013）

- `InputEvent { room_token, client_time_ms, move_x, move_y, move_z, buttons, grab_entity_id, aim_x, aim_y, aim_z }`
- `move_*` はワールド投影済みの正規化 Intent（~-1..1）。Desktop=マウスルック yaw 相対（x0012）、VR=コントローラー aim 相対。host が `move_speed` でスケール
- `buttons` ビット: Primary / Secondary / Grab / Context / Jump（未使用は host 無視可）
- `grab_entity_id` + `aim_*`（ワールド aim 原点: XR コントローラ / Desktop 眼）: GRAB 中の kinematic 追従。`0` / GRAB 無しは無視
- `room_token` は world-server 発行の短命 JWT（検証は host-server）
- `DEFAULT_MOVE_SPEED = 4.0`（Rust 定数。Manifest `move_speed` が 0 のときの host 既定 / Settings 表示）

## キー式ヘルパ

| 定数 / 関数 | 既定 |
|:---|:---|
| `MANIFEST_KEY_LOCAL` / `manifest_key(room_id)` | `alchemy/dev/room/local/manifest` |
| `FRAME_KEY_LOCAL` / `frame_key(room_id)` | `alchemy/dev/room/local/frame` |
| `INPUT_KEY_LOCAL` / `input_key(room_id)` | `alchemy/dev/room/local/input` |
