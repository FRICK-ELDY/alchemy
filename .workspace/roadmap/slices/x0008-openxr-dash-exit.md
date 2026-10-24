# x0008 — OpenXR 最小（ダッシュ＋Exit）

> **ID:** `x0008` · **slug:** `openxr-dash-exit` · **status:** done · **area:** `[client, system_ui]` · **surface:** `[desktop, vr]` · **depends_on:** `[x0007]`

目標: OpenXR ランタイム（確認正本は **Windows + SteamVR**）でセッションを張り、  
**動けなくても** メニューでダッシュを開き **Exit** できる。Desktop（ESC）は壊さない。（完了）

正本: [client.md](../../0_doc/architecture/client.md) · [overview.md](../../0_doc/architecture/overview.md) ① XR 自動判定

## 設計方針

| 層 | 役割 |
|:---|:---|
| boot | ランタイム検出 → XR / Desktop。`ALCHEMY_FORCE_DESKTOP=1` で強制 Desktop |
| surface | Desktop (winit) / OpenXR がデバイス差を吸収 |
| system_ui | retained Canvas + `UiAction` のみ（OpenXR を知らない） |
| XR 提示 | 同一 Canvas を頭部前方ワールドクアッドへ。メニュー＋レイヒット |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice 文書 + 目次 / README / client.md | done |
| 1 | Desktop/XR surface 抽象（system_ui 非依存） | done |
| 2 | OpenXR 検出・セッション・ビュー描画・失敗時 Desktop | done |
| 3 | メニュー開閉・レイヒット・Exit 再利用 | done |
| 4 | WSL CI/Desktop 回帰 + Windows SteamVR 手順 | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| SteamVR OpenXR でセッション起動、失敗時は Desktop | Quest スタンドアロン / WebXR |
| 同一 retained Canvas を頭部前方 CompositionLayerQuad で提示 | XR 内の Coin/floor メッシュ描画（Desktop のみ。XR は backdrop） |
| メニューボタンで開閉、レイヒットで Exit | ロコモーション・予測補間・アバター |
| Desktop ESC 経路を壊さない | 設定タブ中身・マイク実制御 |
| C-C/D/E の frame/manifest/input / system_ui 契約を壊さない | **WSL2 上の実 HMD 表示** / launcher |

## やらない（後続）

- WASD / 親指スティック移動
- Walk / Fly / Noclip / Warp（ワールドロコモーションカタログ）
- launcher 本実装、Quest 単体、Formula、verifier

## 起動（Desktop / WSL）

```bash
# 従来どおり。ランタイムが無ければ Desktop。
cd client
ALCHEMY_FORCE_DESKTOP=1 cargo run -p client   # 明示 Desktop
cargo run -p client                           # 検出して XR 可なら XR
```

## 実機確認（Windows + SteamVR）

README はネイティブ Windows の `cargo` を原則非推奨（os error 4551）としている。  
XR 手動確認だけ Windows ホストで行う。

1. SteamVR を起動し、OpenXR ランタイムを SteamVR にする  
2. リポジトリを Windows から見えるパスに置く（または WSL 外で clone）  
3. アプリ制御（4551）で `cargo` が失敗したら除外パスを追加してから:

```powershell
cd client
cargo run -p client
# 強制 Desktop: $env:ALCHEMY_FORCE_DESKTOP="1"
```

4. HMD 装着 → コントローラ **Menu** でダッシュ開閉  
5. レイで「終了」→「client のみ終了」または「client と host を終了」  
6. host 併用時は従来どおり `ALCHEMY_HOST_PID` または `host_control`

WSL / CI はコンパイル＋Desktop フォールバックのみ保証（`bash .scripts/smoke-openxr.sh`）。

## 実装メモ

- `client/app/src/surface/` — boot 選択・Desktop (winit) / OpenXR (Vulkan enable2)
- `client/app/src/surface/xr/` — セッション・Menu/Select・レイヒット・UI quad アップロード
- `client/app/src/scene.rs` — Desktop 共有シーン GPU
- `client/app/src/system_ui/` — Canvas 据え置き（OpenXR を知らない）
- 環境変数: `ALCHEMY_FORCE_DESKTOP=1`、任意 `ALCHEMY_OPENXR_LOADER`（`openxr_loader.dll` のフルパス）、任意 `STEAM_PATH`
- Steam が D: 等でもレジストリ / `libraryfolders.vdf` / ドライブレター走査でローダーを探索（`surface/xr/loader.rs`）
- smoke: `bash .scripts/smoke-openxr.sh`

← [目次](../README.md)
