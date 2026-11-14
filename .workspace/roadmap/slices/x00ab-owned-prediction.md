# x00ab — 手元予測（Grab Keep·Blend·Snap）

> **ID:** `x00ab` · **slug:** `owned-prediction` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x00aa]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 2

目標: 自分が掴んでいる物はローカル即時予測し、Applied Frame 権威で Keep / Blend / Snap。Grab 拒否は Snap。ヒット・箱/mesh 描画も予測 TRS を使う（`display_pose_for` + `model_override`）。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 予測は毎 input tick；照合は Applied Frame のみ | 毎フレーム predict→reconcile の即 Snap |
| Grab reject → Snap（権威距離ヒューリスティック + grace） | Phase 3 `acked_intent_seq` / `Entity.holder` |
| Keep 時、表示・ヒットが予測から `keep_threshold` 以内 | user_root loco 予測（Epoch eye のまま） |
| `grabbable_display_poses` / box·mesh draw が override を参照 | Epoch GPU storage row の書き換え |

## Grab reject 信号（暫定）

Phase 3 の `acked_intent_seq` / holder までは、掴み開始後の Applied Frame について `held_auth_frames <= reject_grace_frames`（既定 2）は楽観 accept、**3 回目以降**で権威が予測から `blend_threshold` 超 → reject。host が掴みを付けた場合は権威が手元付近に来る。

**再 Grab 仕様**: reject 後は **Grab ボタンを一度離すまで**新規 Grab 禁止（押しっぱなしの grace→reject→再 Grab ループ／Intent 連打を禁止）。

## 完了条件

1. `cargo test -p client owned_prediction` Green。
2. 描画・ヒットが `OwnedPrediction::display_override` を消費する。
3. roadmap 目次で本スライス `done`。
