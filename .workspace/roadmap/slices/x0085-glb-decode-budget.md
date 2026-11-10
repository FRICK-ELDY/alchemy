# x0085 — GLB decode budget（頂点・index・pixel・展開 bytes）

> **ID:** `x0085` · **slug:** `glb-decode-budget` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x0004, x000c, x0048]`  
> **優先度:** 評価 P1-2（正当登録の圧縮爆弾 / 巨大 GLB の OOM 防止）

目標: client の GLB デコードに **入力 bytes・展開 buffers・頂点・index・画像 pixels** のハード上限を設け、超過時は decode `Err`（認証 mesh 経路は既存どおり **placeholder**）にする。Desktop remount と OpenXR render-model の両方で同じ予算を使う。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P1-2** · [gpt-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/gpt/gpt-specific-weaknesses-2026-11-05.md)（`import_slice` 後無制限 Vec）

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `load_*_from_glb` は `AssetDecodeBudget` 標準上限超過で **Err**（認証 GET 経路は placeholder） | assets 側の decode 検疫 |
| 入力超過は `Gltf::from_slice` **前**に拒否。宣言 buffer 長 + 実 buffer + decoded image + **変換成功時のみ** RGBA を `expanded_bytes` に計上 | 単一画像デコード中の `image` クレート内ピークのゼロ化 |
| 頂点 / index は **accessor count を Vec 確保前**に加算して拒否 | |
| 認証 remount（`load_mesh_from_glb`）は **buffers only**（画像デコードしない） | |
| textured 経路は画像 index ごと RGBA を **1 回**変換し `Arc` 共有（primitive 間で `Vec` 増殖しない） | GPU テクスチャの画像 index dedupe（VRAM；decode 予算外） |
| XR: runtime が返す GLB size を `MAX_INPUT_BYTES` 検査してから `Vec` 確保；decode は同一予算 | XR 専用 placeholder |
| Coin / 通常コントローラは標準上限内 | LOD・KTX2・ストリーミング mesh |

---

## 設計（要点）

`client/app/src/asset_decode_budget.rs` + `mesh.rs` 配線。

| 軸 | 標準上限（定数） | 検査タイミング |
|:---|:---|:---|
| `input_bytes` | 32 MiB | parse 前；assets GET `take`；XR size 問合せ後 |
| `expanded_bytes` | 128 MiB | 宣言 buffer 長 → 実 buffer → 各 decoded image → **RGBA は変換成功時のみ** |
| `vertices` / `indices` | 500k / 1.5M | **accessor count**（`Vec` 確保前） |
| `image_pixels` | 16M | 画像ごと decode 直後・保持前（全画像・共有参照は 1 回） |
| RGBA 共有 | — | 画像 index ごとに 1 回だけ RGBA 変換し `Arc` で primitive 間共有 |

テストは `for_test` で input / expanded / vertices / indices / pixels を低予算注入。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / backlog | **done** |
| 1 | `AssetDecodeBudget` + mesh / fetch 配線 | **done** |
| 2 | 否定系 unit（bomb input / 低上限） | **done** |
| 3 | client.md 保証表・improvement 追記 | **done** |

---

## 完了条件

1. 標準上限超過の入力 bytes → `Gltf::from_slice` 前に Err
2. 低予算での頂点/index/pixels 超過 → Err（巨大 fixture をメモリに載せず検証可能）
3. Coin fixture は標準予算で緑
4. `load_auth_mesh` の decode Err → placeholder（既存経路）
5. architecture 保証表更新；`cargo test -p client` mesh 関連緑

## やらない（本 Slice）

- ~~assets test hermetic（P1-3）~~ → [x0086](x0086-assets-test-hermetic.md)
- WAN host E2E（P1-4）
- assets-server 側の展開検疫ヒューリスティック（Proprietary 境界）

## 関連

- [x0048-asset-placeholder-http-timeout.md](x0048-asset-placeholder-http-timeout.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
