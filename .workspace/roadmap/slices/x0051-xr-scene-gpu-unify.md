# x0051 — XR メッシュ描画を SceneGpu に一本化（P4-1 / 2/3）

> **ID:** `x0051` · **slug:** `xr-scene-gpu-unify` · **status:** `planned` · **area:** `[client, docs]` · **depends_on:** `[x0050]`  
> **優先度:** **P4-1**（描画経路統合・第2スライス）

目標: XR のワールド描画を Desktop と同じ **wgpu `SceneGpu`** 経路に寄せる。stereo は **眼ごと 2 pass**（左右の `view`/`proj`）。ash 手書きレンダはフラグ切替で残してよい（削除は x0052）。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P4-1** · 正本 [client.md](../../0_doc/architecture/client.md) · 先行 [x0050](x0050-xr-swapchain-wgpu.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| XR が SceneGpu 相当の内容を表示（floor / coin mesh / boxes + highlight） | ash コードの完全削除（→ x0052） |
| XR 固有オーバーレイ（UI canvas quad / laser / controller GLB）を wgpu 側へ移設 | LOD / KTX2 / スキン / PBR |
| stereo は dual-eye pass（`draw_world` 相当 × 2） | SceneGpu の multiview パイプライン化 |
| Desktop 経路の回帰なし | CompositionLayerQuad への UI 分離 |
| 保証表とコード一致 | Bevy 導入 |

## 設計（要点）

1. x0050 の swapchain wgpu Texture に対し、左右 `TextureView`（array layer 0/1）で 2 pass
2. 既存 [`SceneGpu::draw_world`](../../../client/app/src/scene.rs) を XR から呼ぶ（必要なら XR 用オーバーレイ API を近傍に追加）
3. SystemUi の wgpu／CPU canvas 経路を再利用できるものは寄せる
4. 一時フラグ（`ALCHEMY_XR_WGPU` 等）で ash 旧経路と切替可能にしておく

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | planned |
| 1 | SceneGpu dual-eye（floor / mesh / boxes） | planned |
| 2 | UI canvas / laser / controller を wgpu へ | planned |
| 3 | Desktop 回帰確認 + client.md | planned |

## 完了条件

1. フラグ ON（または既定切替）で XR が箱・メッシュ・ダッシュ UI・レーザー／コントローラを表示
2. Desktop 経路の見た目・操作に回帰なし
3. architecture 保証表とコード一致

## やらない

- ash パイプライン／naga→SPIR-V multiview の削除
- 一時フラグの撤去（→ x0052）

← [目次](../README.md) · 前: [x0050](x0050-xr-swapchain-wgpu.md) · 次: [x0052](x0052-xr-vk-loop-gpu-remove.md)
