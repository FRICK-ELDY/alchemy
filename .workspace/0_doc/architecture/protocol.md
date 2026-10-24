# protocol（alchemy-protocol）

> **正本**: [`protocol/`](../../../protocol/)  
> **PROTOCOL_PIN**: `alchemy-protocol/0.1.0-slice-a`

## 置き場の方針

スーパープロジェクト直下の **独立クレート**。client submodule 化後もここ（または将来の `alchemy-protocol` リポ）に置き、client / host-server / Elixir サーバが参照する。

client workspace からは `path = "../protocol"` で依存する（`client` の中には埋め込まない）。

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| `RenderFrame` protobuf の encode / decode | デルタ圧縮 |
| `PROTOCOL_PIN` による世代識別 | フル DrawCommand 体系 |
| f64 空間座標（描画境界） | audio / tracking / input_events（後続） |

## Slice A メッセージ

`proto/render_frame.proto`:

- `RenderFrame { tick, host_time_ms, entities[] }`
- `Entity { id, x,y,z (double), r,g,b,a (float) }`

host-server が Publish、client が Subscribe する唯一の空間同期ペイロード。
