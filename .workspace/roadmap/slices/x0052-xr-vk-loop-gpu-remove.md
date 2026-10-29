# x0052 — XR `vk_loop` GPU 部削除（P4-1 / 3/3）

> **ID:** `x0052` · **slug:** `xr-vk-loop-gpu-remove` · **status:** `planned` · **area:** `[client, docs]` · **depends_on:** `[x0051]`  
> **優先度:** **P4-1**（描画経路統合・第3スライス）

目標: ash 手書きレンダを削除し、XR ループを「セッション＋入力＋swapchain acquire/release＋wgpu submit」の薄層にする。wgpu 経路を既定化し、評価上の描画二重化（`-2`）を閉じる。

根拠: [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P4-1** · 正本 [client.md](../../0_doc/architecture/client.md) · 先行 [x0051](x0051-xr-scene-gpu-unify.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| XR ビルドが ash **グラフィックスパイプライン**に依存しない | OpenXR+Vulkan enable／wgpu-hal interop 用の ash 残存（必要最小） |
| Desktop / XR のワールド描画ロジックが SceneGpu 系で単一 | ファイル名必須リネーム（`frame_loop.rs` 等は任意） |
| 一時フラグ（clear／旧 ash 切替）を整理し wgpu を既定化 | Formula / LOD / 予測補間 |
| roadmap / client.md / improvement-plan の P4-1 クローズ記載 | verifier / launcher 本実装 |

## 設計（要点）

1. `create_*_pipeline` / naga→SPIR-V multiview / `upload_gpu_*` 等の ash 描画ヘルパを削除
2. [`vk_loop.rs`](../../../client/app/src/surface/xr/vk_loop.rs) をセッション・入力・present 中心に薄くする（リネームは任意）
3. `ALCHEMY_XR_WGPU` 等の移行用フラグを撤去または no-op 化
4. 文書: client.md 保証表、improvement-plan P4-1、本スライス status=done

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | planned |
| 1 | ash 描画コード削除・wgpu 既定化 | planned |
| 2 | フラグ整理・モジュール整理 | planned |
| 3 | docs / P4-1 クローズ + CI 緑 | planned |

## 完了条件

1. XR が SceneGpu 系のみで描画（ash graphics pipeline なし）
2. Desktop / XR 回帰なし、`cargo test -p client` / CI 緑
3. 評価「描画二重化 -2」を閉じる根拠が文書とコードで揃う

## やらない

- Bevy 移行
- multiview 再導入（dual-eye のまま可）
- P4-2 launcher / P4-3 verifier

← [目次](../README.md) · 前: [x0051](x0051-xr-scene-gpu-unify.md)
