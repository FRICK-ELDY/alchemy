# x0011 — Grab hold（kinematic 追従）

> **ID:** `x0011` · **slug:** `grab-hold` · **status:** done · **area:** `[host, client, protocol, sim]` · **surface:** `[vr]` · **depends_on:** `[x0010]`

目標: x0010 の Grabbable ボックスを、ダッシュ閉時の aim レーザー＋**Grab 押下**で **kinematic 追従**し、離したら dynamic に戻して放下する。Steal / Desktop Grab / 回転追従はしない。

正本: [client.md](../../0_doc/architecture/client.md) · [protocol.md](../../0_doc/architecture/protocol.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| `InputEvent` | `buttons.GRAB` + `grab_entity_id` + ワールド `aim_*`（コントローラ aim 原点） |
| client XR | ダッシュ閉・レーザーヒット中に Grab 開始。保持中はヒット外れても同一 entity を送り続ける |
| host | Grab 開始で dynamic→kinematic、`aim + offset` を `set_next_kinematic_translation`。離したら dynamic |
| offset | 開始時 `box - aim` を固定（リモート掴みでもスナップしない） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | proto: `grab_entity_id` + `aim_*` + PROTOCOL_PIN | done |
| 2 | XR: Grab 押下→Intent、保持・放下 | done |
| 3 | host: kinematic 追従 + 放下 | done |
| 4 | smoke + docs | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| XR・ダッシュ閉時、aim ヒット中の Grab で entity 10..18 を掴める | Desktop Grab / マウス |
| 保持中はコントローラ aim 原点＋開始時 offset で kinematic 追従 | 回転追従・スケール・2 ハンド |
| 離すと dynamic に戻り重力下で落下／着地 | Steal / 他クライアント奪取 / 予測補間 |
| `grab_entity_id==0` または非 grabbable は無視 | Grip 以外の専用ツール・Context |
| system_ui / Dash は Grab を知らない | WorldRoot UI・Facet 並べ替え |

## やらない（後続）

- Desktop Grab / Pointer Lock → **x0012**（look）・**x0013**（Grab）
- Steal / Receivable / Scalable（Resonite 互換フィールド）
- 掴み中の角速度／投げ（release impulse）
- 汎用 Slot 親子付け

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server
# Windows + SteamVR（WSL は HMD 非保証）
cargo run -p client
```

確認:

1. ダッシュ閉 → レーザーをボックスに当てて Grip（なければ Select）押下で箱が手に追従  
2. 離すと箱が落ちる／床に着く  
3. ダッシュ開中は Grab せず UI Select のまま  

## 実装メモ

- `protocol/proto/input_events.proto` — `grab_entity_id` / `aim_x|y|z`
- `client/app/src/scene.rs` — `MoveIntent` 拡張
- `client/app/src/surface/xr/vk_loop.rs` — held entity + world aim
- `client/host-server/src/sim.rs` — grab begin/hold/release
- smoke: `bash .scripts/smoke-rapier-host.sh`

← [目次](../README.md)
