# x0009 — Desktop Noclip ロコモーション

> **ID:** `x0009` · **slug:** `desktop-noclip-loco` · **status:** done · **area:** `[protocol, host, client]` · **surface:** `[desktop]` · **depends_on:** `[x0008]`

目標: World 定義（`SceneManifest.locomotion`）の **既定 Noclip** で、Desktop WASD 移動を host 権威で動かす。  
client は mode 非依存の Intent のみ送り、VR ロコモーションは後続。（完了）

正本: [client.md](../../0_doc/architecture/client.md) · [protocol.md](../../0_doc/architecture/protocol.md) · [Resonite LocomotionController](https://wiki.resonite.com/Component:LocomotionController)（参考）

## 設計方針

| 層 | 役割 |
|:---|:---|
| World 定義 | `SceneManifest.locomotion`（低頻度）。UNSPECIFIED → host は Noclip |
| client Desktop | WASD / Space / C → `InputEvent` Intent。ダッシュ中は 0。マウスルックは後続 |
| host | Manifest mode を実行。Noclip は衝突無視で `user_root` を積分 |
| 描画 | Coin（entity 1）と `user_root`（entity 3）を分離。カメラは user_root＋固定 look |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 / PROTOCOL_PIN | done |
| 1 | `LocomotionConfig` + InputEvent buttons | done |
| 2 | host: user_root Noclip 積分 | done |
| 3 | Desktop Intent + カメラ追従 | done |
| 4 | smoke + docs | done |

## Desktop バインド

| 入力 | 役割 | 本 Slice |
|:---|:---|:---|
| Esc | ダッシュ開閉 | ダッシュ中は Intent = 0 |
| WASD | 前/左/後/右（固定 look） | 実装 |
| Space | Noclip/Fly: 上昇 / Walk: ジャンプ | Noclip 上昇 |
| C | Noclip/Fly: 下降 | 実装 |
| マウス移動 / Pointer Lock | yaw/pitch | **後続**（WSL で不安定のため一旦撤去） |
| Y / T / R | Primary / Context / Secondary | キースタブ |
| LMB | ダッシュ UI | ダッシュ開時のみ |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Manifest 既定 Noclip、Desktop WASD+Space+C ですり抜け移動 | VR ロコモーション / マウスルック |
| `user_root` pose がデルタで届きカメラが追従 | Grab / Context / Primary ツール本体 |
| Intent はワールド mode 非依存 | 予測補間・本格 Walk キャラコン |
| Room Token + `is_finite` ゲート維持 | モジュール切替 UI・抑制ソース |

## やらない（後続）

- Desktop マウスルック / Pointer Lock
- VR 親指スティック
- Grab / コンテキストメニュー / 装備ツール
- Resonite 型モジュール一覧＋ ActiveModuleIndex UI
- world-server 署名 Manifest / verifier

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

## 実装メモ

- `protocol/proto/scene_manifest.proto` — `LocomotionConfig` / `LocomotionMode`
- `protocol/proto/input_events.proto` — Intent 文書化 + `buttons`
- `PROTOCOL_PIN` = `alchemy-protocol/0.4.0-x0009`
- `client/host-server/src/sim.rs` — Coin 物理と user_root Noclip 分離
- `client/app/src/surface/desktop.rs` — Intent サンプリング
- entity: Coin=`1`、static marker=`2`、user_root=`3`
- smoke: `bash .scripts/smoke-desktop-noclip.sh`

← [目次](../README.md)
