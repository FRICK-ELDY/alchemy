# protocol（alchemy-protocol）

> **正本**: [`protocol/`](../../../protocol/)  
> **PROTOCOL_PIN**: `alchemy-protocol/0.3.0-slice-c-c`

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

## メッセージ

### SceneManifest（`proto/scene_manifest.proto`）— Slice C-C・低頻度

- `SceneManifest { room_id, revision, entries[] }`
- `SceneEntry { entity_id, blake3 (hex), x,y,z }`
- **GLB バイトは載せない**（assets-server CAS 参照のみ）

### RenderFrame（`proto/render_frame.proto`）— Slice C-C・高頻度デルタ

- `RenderFrame { tick, host_time_ms, upserts[], removes[] }`
- `Entity { id, x,y,z (double), r,g,b,a (float) }`
- 静止 Entity は `upserts` に載せない（フルスナップショット廃止）

### InputEvent（`proto/input_events.proto`）— Slice B

- `InputEvent { room_token, client_time_ms, move_x, move_y, move_z }`
- `room_token` は world-server 発行の短命 JWT（検証は host-server）

## キー式ヘルパ

| 定数 / 関数 | 既定 |
|:---|:---|
| `MANIFEST_KEY_LOCAL` / `manifest_key(room_id)` | `alchemy/dev/room/local/manifest` |
| `FRAME_KEY_LOCAL` / `frame_key(room_id)` | `alchemy/dev/room/local/frame` |
| `INPUT_KEY_LOCAL` / `input_key(room_id)` | `alchemy/dev/room/local/input` |
