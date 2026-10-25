# x000a — VR 親指スティック Noclip

> **ID:** `x000a` · **slug:** `vr-thumbstick-noclip` · **status:** done · **area:** `[client, host, protocol]` · **surface:** `[vr]` · **depends_on:** `[x0009]`

目標: OpenXR 親指スティックを **コントローラー aim 相対** で world Intent に射影し、既存 host Noclip で `user_root` を動かす。  
倒し量で速度、左右加算、Settings に既定 max speed 表示（編集は後続）。

正本: [client.md](../../0_doc/architecture/client.md) · [protocol.md](../../0_doc/architecture/protocol.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| client OpenXR | 左右 thumbstick + aim pose → `MoveIntent`。ダッシュ中は 0 |
| 射影 | stick X=右、Y=前方（-local Z）。ピッチ込みで上下も出る |
| 合成 | L+R 加算 → 長さ >1 なら正規化。`max_speed/DEFAULT` スケール |
| host | 既存 `apply_user_intent`（ワイヤ変更なし） |
| Settings | `Max speed: 4.0 m/s` 読み取り専用（仮想キーボードは後続） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | `DEFAULT_MOVE_SPEED` 共有 + loco 純関数 | done |
| 2 | OpenXR thumbstick + Intent | done |
| 3 | Settings 既定値表示 | done |
| 4 | smoke + docs | done |

## VR バインド

| 入力 | 役割 | 本 Slice |
|:---|:---|:---|
| Menu | ダッシュ開閉 | 既存（ダッシュ中 Intent=0） |
| Select + aim | ダッシュ UI クリック | 既存 |
| 左右 thumbstick | aim 相対移動（倒し量・加算） | 実装 |
| Settings | max speed 既定値表示 | 読み取り専用 |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| SteamVR + Index/Touch 系でスティック Noclip | Vive トラックパッド専用 / ホバー・テレポート |
| aim 相対・倒し量・左右加算・ダッシュ中 0 | マウスルック / Walk キャラコン |
| Settings に既定 max speed 表示 | 仮想キーボード編集・永続化・host 速度上書き |
| Desktop WASD / host Noclip 経路維持 | WSL 実 HMD |

## やらない（後続）

- 仮想キーボードによる max speed 編集
- Desktop マウスルック / Pointer Lock
- Grab / Context / Primary ツール本体
- Vive トラックパッド専用マップ

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

## 実装メモ

- `protocol` — `DEFAULT_MOVE_SPEED` 公開（`PROTOCOL_PIN` 据え置き）
- `client/app/src/surface/xr/loco.rs` — stick→world / 加算 / スケール
- `client/app/src/surface/xr/vk_loop.rs` — thumbstick actions + Intent
- `client/app/src/system_ui/dash.rs` — Settings 既定値表示
- smoke: `bash .scripts/smoke-openxr.sh`

← [目次](../README.md)
