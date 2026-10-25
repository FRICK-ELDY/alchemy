# x0012 — Desktop マウスルック（Pointer Lock）

> **ID:** `x0012` · **slug:** `desktop-mouse-look` · **status:** done · **area:** `[client]` · **surface:** `[desktop]` · **depends_on:** `[x0009]`

目標: Desktop・ダッシュ閉時に **Pointer Lock（失敗時 Confined）** でマウス Δ → クライアント局所の yaw/pitch を更新し、カメラと **WASD Intent の水平向き**を揃える。ワイヤ schema は変えず、host は従来どおり世界投影済み `move_*` のみ見る。（完了）

正本: [client.md](../../0_doc/architecture/client.md) · [protocol.md](../../0_doc/architecture/protocol.md)

## 設計方針

| 層 | 役割 |
|:---|:---|
| `CameraLook`（client 局所） | yaw / pitch。host / Zenoh に載せない |
| surface Desktop | ダッシュ閉＋クリックで grab；Esc でダッシュ開＋解放；`DeviceEvent::MouseMotion` で Δ |
| Intent | WASD を **現在 yaw** 相対で世界投影（x0009 の yaw=0 固定を解除） |
| host | 変更なし（`move_*` 積分のみ） |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 | done |
| 1 | Pointer Lock / Confined + Esc 解放 + focus 喪失解放 | done |
| 2 | MouseMotion → `CameraLook`（pitch clamp） | done |
| 3 | WASD を look yaw 相対に | done |
| 4 | smoke + client.md / README | done |

## Desktop バインド（本 Slice 追加）

| 入力 | 役割 |
|:---|:---|
| LMB（ダッシュ閉） | カーソル grab（`Confined` → 失敗時 `Locked`）＋非表示。Linux/WSL は Confined 優先 |
| マウス移動（grab 中） | `CursorMoved` Δ（recenter）および／または `DeviceEvent` → yaw / pitch |
| Esc | ダッシュ開閉。**開くとき** grab 解放＋カーソル表示 |
| フォーカス喪失 | grab 解放＋カーソル表示 |

ダッシュを閉じただけでは再ロックしない（クリックで明示。WSL での誤掴みを避ける）。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| ダッシュ閉・grab 成功時、マウス Δ でカメラ yaw/pitch が変わる | VR / OpenXR の変更 |
| WASD 水平移動が現在の look yaw 相対になる | pitch を Intent の上下に混ぜる（Space/C は従来どおり世界 Y） |
| Esc でダッシュを開くと grab が外れ UI マウスが使える | raw `DeviceEvent` 単独依存（WSL では Confined+CursorMoved を正とする） |
| look は client 局所（Room / host に載せない） | Desktop Grab / 感度設定 UI / マウス加速プロファイル |

## やらない（後続）

- Desktop Grab / ワールドクリック選択
- 感度・反転の Settings Facet
- 予測補間・host 側向き同期
- VR 回転追従 / 投げ

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5
cd client
cargo run -p host-server
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client
```

確認:

1. ダッシュ閉で画面クリック → カーソルが消える（または Confined）  
2. マウスを動かすと視点が回り、W が視線水平方向に進む  
3. Esc でダッシュ → カーソル復帰・UI クリック可。閉じて再度クリックするまで look 固定  

## 実装メモ

- `client/app/src/surface/desktop.rs` — grab / MouseMotion / yaw 相対 Intent
- `client/app/src/scene.rs` — 既存 `CameraLook` + `desktop_view_proj`（pitch clamp 維持）
- proto フィールド追加なし（コメントのみ Desktop look を反映可）
- smoke: `bash .scripts/smoke-desktop-noclip.sh`（ホスト側回帰）＋任意で look ヘルパ単体テスト

← [目次](../README.md)
