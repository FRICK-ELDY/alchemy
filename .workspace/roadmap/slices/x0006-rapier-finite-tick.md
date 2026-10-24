# x0006 — Rapier 最小＋ `is_finite`（内部 60Hz / 配信 30Hz）（完了）

> **ID:** `x0006` · **旧称:** Slice D · **slug:** `rapier-finite-tick` · **status:** done · **area:** `[host, sim]` · **depends_on:** `[x0005]`

目標: host-server を「デモ軌道」から **空間計算ホスト** に引き上げる。  
内部物理 Tick（60Hz 固定）とネットワーク配信 Tick（30Hz・C-C デルタ維持）を分離し、入力は `is_finite` 強制サニタイズ後にだけ物理へ渡す。  
SceneManifest / assets 経路は C-C のまま（GLB は host 非経由）。

正本: [client.md](../../0_doc/architecture/client.md) · [overview.md](../../0_doc/architecture/overview.md) ⑧（3 層 Tick の ①②）

## 設計方針

| 層 | 周波数 | 役割 | 通信 |
|:---|:---|:---|:---|
| 内部物理 Tick | 60Hz 固定 | Rapier 積分・衝突（単精度 f32 状態） | なし |
| 配信 Tick | 30Hz（C-C） | 動いた Entity だけ `RenderFrame` デルタ Publish（描画境界は f64） | Zenoh `…/frame` |
| 入力ゲート | 受信時 | Room Token 検証 + `is_finite`（NaN/Inf は drop） | Zenoh `…/input` |
| Manifest | 低頻度（C-C） | `entity_id → blake3` | Zenoh `…/manifest` |

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | host: `is_finite` サニタイズ（座標・速度系入力）。非 finite は drop + ログ | done |
| 1 | host: Rapier3d 最小ワールド（床 + 動く剛体 1）。内部 60Hz step | done |
| 2 | host: 物理 pose → 既存デルタ Publish（静止スリープ維持）。デモ円軌道を撤去 | done |
| 3 | smoke-rapier-host + unit tests + docs | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 入力の非 finite（NaN/Inf）を物理に入れない | Formula VM / Dual Editor ギミック |
| 内部 60Hz と配信 30Hz の分離 | 可変配信レート UI（20/60Hz 切替） |
| Rapier で少なくとも床衝突する 1 動体の pose がデルタで届く | 本格キャラコン・ジョイント・連続CCD 全般 |
| C-C の Manifest / デルタ契約を壊さない（`PROTOCOL_PIN` 据え置き） | client 予測補間・verifier・署名 Manifest |
| | 公式リレー / launcher / LOD |

## やらない（後続）

- Formula VM / Dual Representation
- world-server 署名付き Scene Manifest
- client 側 60–120fps 予測補間の本格化
- verifier Spot-Check / ホスト移譲
- `shared-sim` クレートの本格切り出し（同一 host 内 `sim` モジュールで開始）

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
cd world-server && docker compose up -d && cd ..
bash .scripts/smoke-assets-glb.sh
zenohd -c zenoh-router/config.json5
bash .scripts/smoke-rapier-host.sh
cd client
cargo run -p host-server   # 60Hz Rapier → 30Hz デルタ + Manifest
cargo run -p client        # C-C 経路のまま pose 追従
```

実装メモ:

- `client/host-server/src/finite.rs` — `is_finite` ゲート
- `client/host-server/src/sim.rs` — Rapier 床 + 動的ボール（entity 1）
- 有限入力は水平 `linvel`（XZ）のみ適用し、Y は重力に任せる（キーボード操作は後続）
- 時間回転描画はしない（物理 pose を隠さない）。床平面＋高さで色変化

← [目次](../README.md)
