# x0091 — Epoch `out` → SceneGpu draw（vertex/storage）

> **ID:** `x0091` · **slug:** `epoch-out-scenegpu-draw` · **status:** `done` · **area:** `[client, sim]` · **depends_on:** `[x0090]`  
> **優先度:** Zero-Base Spatial 本線（x0090 の「次」）

目標: `sync_epoch_gpu` の interpolate 結果 `out` を SceneGpu の mesh/box 描画が **GPU readback なし**で読む。画面上の TRS 権威を wire Curr `EntityPose` から **Epoch display 面（α / ABA）** へ移す。Grab OBB も同じ面に揃える。

根拠: [x0090](x0090-client-epoch-scenegpu-bind.md)「やらない／次」· [alchemy_zero_base_spatial_lifecycle_architecture.md](../alchemy_zero_base_spatial_lifecycle_architecture.md) Step 5

---

## 方針（レガシー撤廃 → 新方式）

| 方式 | 内容 | 扱い |
|:---|:---|:---|
| **レガシー（撤廃）** | 画面に見えるもの／掴むものの TRS 権威を wire **Curr `EntityPose` のみ**にする。描画だけ α 補間して Grab/ヒットは Curr のまま、など **見た目と入力の面を分ける** | **採用しない**。x002b「hit matches draw TRS」および Zero-Base Ring3 の知覚面契約と矛盾する |
| **新方式（正）** | **Display TRS** = 表示フレーム先頭で **一度だけ** Prev/Curr + α を固定（`begin_display_frame`）。GPU upload・Grab OBB・コイン色 Y はすべてその snap を読む。readback なし | **SSoT**。フレーム内の別 `Instant::now()` / 再 `load_pair` は禁止 |
| **残レガシー（後続で撤廃）** | カメラ／loco **eye** がまだ WorldView Curr | x0091 では未配線。**長期方針ではない**。次 Slice で display 面へ移す（本 Slice の「保証しない」に残すのは実装未了の記録） |

採択メモ（レビュー回答）: 「描画だけ補間・Grab は Curr」は x0091 の意図ではない。Grab/ヒットは新方式必須。eye の Curr 据え置きは暫定残債であり、レガシー方式の延長として残してはならない。

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 表示フレームの mesh/box（＋remote root）TRS は `epoch_out` storage 経由（補間後） | GPU → CPU readback |
| Grab / コイン Y / GPU は `begin_display_frame` の同一 snap | **eye の display 化**（残レガシー・後続撤廃） |
| pack は **フル active**；pack index は **packed Curr** から構築 | dirty-only の帯域最適化 |
| `owner_scope` → pack index；未登録は `EntityPose` model_override フォールバック | topology 親子・色の tape 化 |
| floor は従来どおり CPU model（`pack_index=MAX`） | XPBD wire 権威 / Rapier 削除 |
| | Hierarchy UI / Multi-Draw Indirect |

---

## 設計（要点）

```
begin_display_frame (1× / display frame, Grab より前)
  load_pair 1回 + published_at_ns から α + full-active pack + pose cache
        │
        ├─► Grab / laser / highlight / coin Y   ← 同一 snap
        │
        └─► sync_epoch_gpu
              upload 同一 prev/curr rows + 同一 α → compute → out
                    │
                    ▼
              draw_world (epoch_out[pack_index])
```

- **質問への答え:** はい。α／PrevCurr は表示フレーム固定で一度だけ確定し、Grab・色・CPU 双子・GPU はそれを読むのが正。
- Snap は `Arc<DisplayFrame>`（読者はポインタ clone のみ）。Redraw 外の Grab は直前 paint の freeze を `ensure` で再利用。
- CPU lerp キャッシュと GPU compute の二重適用は readback 回避のための意図的二重経路（将来 MDI で整理可）。
- `out` は compute の `read_write` と vertex の `read` をパス順で共有（同一 encoder 内の暗黙バリア）。
- コイン傾き（0.25 rad X）は `extra_rot` として client-local のまま。
- 色チャンネルは tape 未搭載のため wire のまま；高さグラデ Y は **同一 freeze** の display pose。
- wire Curr `EntityPose` は **受理・カメラ暫定・色チャンネル** 用。画面 TRS / Grab の権威には使わない（上表「レガシー撤廃」）。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 | **done** |
| 1 | shader + Uniforms + bind（out storage） | **done** |
| 2 | sync フル active + entity→pack 地図 / draw 配線 | **done** |
| 3 | 単体 + `cargo test -p client` / docs 完了条件 | **done** |

---

## 完了条件

1. `cargo test -p client` 緑（pack 地図・回帰含む）
2. mesh/box 描画パスが `epoch_out` を bind していること（差分レビュー）
3. `sync_epoch_gpu` が `dirty_only=false`（フル active）
4. GPU readback を追加していないこと
5. host wire / Rapier 権威を変えていないこと

## やらない（本 Slice）／次

- **カメラ／loco eye を display TRS へ移す**（残レガシー撤廃・次候補）
- XPBD を wire 権威にする / Rapier 削除
- Loro CRDT / Zenoh Grab ストリーム
- UI Lens 本仕様 / Hierarchy 本配線
- wire pose ダブルバッファ置換（improvement P2-6 残り）
- Multi-Draw Indirect / 永続 GPU スロット freelist

## 関連

- [x0090-client-epoch-scenegpu-bind.md](x0090-client-epoch-scenegpu-bind.md)
- [x008e-zero-base-spatial-lifecycle.md](x008e-zero-base-spatial-lifecycle.md)
- [x008f-host-frame-bridge.md](x008f-host-frame-bridge.md)

← [目次](../README.md)
