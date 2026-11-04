# x005b — Slot 操作のセッション内 undo/redo

> **ID:** `x005b` · **slug:** `slot-session-undo` · **status:** `done` · **area:** `[sim, host, client]` · **depends_on:** `[x005a]`  
> **ladder:** Wave 2 · [inventory-ladder.md](../inventory-ladder.md)

目標: Resonite 相当の **手元コンテキストメニュー undo/redo** を保証する。空間操作（例: 物体を A→B へ移動／渡す／置く）の直後、手元から undo すると **B→A** に戻り、redo すると **A→B** に再適用される。永続履歴・VCS ではない。

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 操作者視点で、直近の空間遷移（reparent / pass / place 等）を undo で逆変換・redo で再適用できる | 退出後の履歴保持・他セッションへの同期 |
| 遷移は逆変換可能（例: prev_parent / 元 Slot を記録） | 物理速度・乱数・他ユーザ操作までの完全再現 |
| 手元コンテキスト（または同等の個人 UI）から undo/redo を選べる入口 | Resonite コンテキストメニュー全体の複製 |
| スタック上限と溢時の破棄方針が文書化される | Git Commit / Spatial Diff / Vcs Rollback |
| host が権威として遷移を適用し、client は入口 UI のみ（定義 vs 実行） | 共同編集 CRDT・他人の操作の無断 undo |

## 設計メモ

- **語彙**: 「session undo」と Vcs Rollback（x0064）を混ぜない
- **実装**: `alchemy-shared-sim::SlotUndoStack`（cap 既定 32・溢時 drop-oldest）
- **入口 UI**: 本 Slice はスタック API + 単体テストまで。Dash 手元メニュー配線は後続で可

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice（Resonite 相当の意味を固定） | done |
| 1 | 遷移レコード + undo/redo + tests（A→B / B→A） | done |
| 2 | 手元コンテキスト入口（最小）+ host 配線 | deferred（API done・UI 後続） |

## 完了条件

1. A→B undo→A / redo→B の単体テスト緑
2. cap 溢時に oldest 破棄

## やらない

- Vcs Rollback
- 他ユーザ操作の undo

← [目次](../README.md) · [ladder](../inventory-ladder.md)
