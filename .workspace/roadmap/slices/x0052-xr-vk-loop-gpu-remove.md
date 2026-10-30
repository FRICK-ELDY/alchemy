# x0052 — XR `vk_loop` GPU 部削除（P4-1 / 3/3）

> **ID:** `x0052` · **slug:** `xr-vk-loop-gpu-remove` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x0051]`  
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

1. `vk_loop.rs`（ash create_*_pipeline / naga→SPIR-V multiview / upload_gpu_*）を削除
2. [`wgpu_session.rs`](../../../client/app/src/surface/xr/wgpu_session.rs) を XR 既定経路に（`mod.rs` 直結）
3. `ALCHEMY_XR_WGPU` フラグ撤去
4. `naga` 依存削除（ash graphics 専用だった）
5. 文書: client.md 保証表、improvement-plan P4-1、本スライス status=done

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | ash 描画コード削除・wgpu 既定化 | done |
| 2 | フラグ整理・モジュール整理 | done |
| 3 | docs / P4-1 クローズ + `cargo test -p client` / clippy 緑 | done |

## 検証根拠（CI vs HMD）

| ゲート | 保証すること | 保証しないこと |
|:---|:---|:---|
| `cargo clippy -p client -- -D warnings` / `cargo test -p client` | 単一経路がコンパイルされ、`surface/xr` 単体テストが通る | HMD 実機の見た目 |
| `.scripts/smoke-openxr.sh`（`ALCHEMY_FORCE_DESKTOP=1`） | CI/WSL で HMD なしに client をビルド・テストできる | XR present／stereo 実行 |
| 運用者 HMD 目視 | SceneGpu floor/mesh/boxes + dash/laser/controllers | CI 自動化 |

## 完了条件

1. XR が SceneGpu 系のみで描画（ash graphics pipeline なし）
2. Desktop 回帰なし、`cargo test -p client` / clippy 緑（HMD 実行は運用者正本）
3. 評価「描画二重化 -2」を閉じる根拠が文書とコードで揃う（[specific-weaknesses](../../0_doc/evaluation/specific-weaknesses-2026-10-28.md) もクローズ注記）

## やらない

- Bevy 移行
- multiview 再導入（dual-eye のまま可）
- P4-2 launcher / P4-3 verifier

← [目次](../README.md) · 前: [x0051](x0051-xr-scene-gpu-unify.md)
