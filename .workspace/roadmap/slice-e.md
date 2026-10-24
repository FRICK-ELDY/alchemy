# Slice E — system_ui ダッシュ（自前 Canvas）（完了）

目標: Desktop / 将来 VR で共通の **1 Canvas** ダッシュ（Resonite 風）を最小導入する。  
ESC で開閉、Material Icons 静的同梱、終了フロー（client 保証 / host は PID・制御スタブ）。

正本: [client.md](../0_doc/architecture/client.md) · [overview.md](../0_doc/architecture/overview.md) ② `system_ui`

## 設計方針

| 層 | 役割 |
|:---|:---|
| retained レイアウト木 | Rect / 色 / アイコン / ラベル。dirty 時のみ再構築 |
| Canvas テクスチャ | 同じ木 → 同じ描画結果（Desktop オーバーレイ / 将来 VR クアッド） |
| 入力 | ESC トグル + マウスヒット（VR メニューボタンは後続） |
| host 終了スタブ | `ALCHEMY_HOST_PID` の SIGTERM、または Zenoh `…/host_control` |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | `slice-e.md` + 目次 / client.md / README | done |
| 1 | Canvas 基盤（レイアウト・ヒット・wgpu オーバーレイ） | done |
| 2 | ESC トグル + マウス | done |
| 3 | 上下バー＋終了/設定、Material Icons | done |
| 4 | 終了アクション（client / host スタブ） | done |
| 5 | unit tests + 手順 | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| ESC でダッシュ開閉（マウスヒット） | OpenXR / VR メニューボタン本実装 |
| 上下バー＋終了パネルの最小レイアウト（1 Canvas） | 設定タブ中身・マイク実制御・同期本実装 |
| 時刻・FPS 表示（ローカル計測） | World/Friends/Inventory 等の全タブ |
| 「client のみ終了」でプロセス終了 | launcher 本実装・ホスト移譲 |
| 「client+host 終了」スタブ（`ALCHEMY_HOST_PID` / `host_control`） | 確実なリモート host kill / 複数 host |
| Material Icons 静的同梱 | テーマエンジン・ロケール本格 |
| C-C/D の frame/manifest/input 契約を壊さない | Formula / 予測補間 / verifier / WASD→Rapier |

## やらない（後続）

- WASD → Rapier 入力ループ
- 設定画面の中身、マイク/同期の実機能
- OpenXR、予測補間、launcher、Formula、署名 Manifest

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
zenohd -c zenoh-router/config.json5

# 別ターミナル: host（PID を控えると client+host 終了スタブが使える）
cd client
cargo run -p host-server
# export ALCHEMY_HOST_PID=<pid>

# 別ターミナル: client — ESC でダッシュ
cargo run -p client
```

操作:

- `Esc` — ダッシュ開閉
- 下バー「終了」— 中央に終了パネル
- 「client のみ終了」— client プロセス終了（host は残る）
- 「client と host を終了」— client 終了 + `ALCHEMY_HOST_PID` へ SIGTERM、なければ Zenoh `alchemy/dev/room/local/host_control` に `shutdown`

実装メモ:

- `client/app/src/system_ui/` — retained Canvas + ダッシュ定義
- `client/app/assets/icons/` — Material Icons（Apache-2.0）
- `client/app/src/host_control.rs` — host 終了スタブ（UI から隔離）
- host-server は `host_control` Subscribe → `shutdown` で exit

← [目次](README.md)
