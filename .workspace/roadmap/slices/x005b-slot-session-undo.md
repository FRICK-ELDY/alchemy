# x005b — Slot 操作のセッション内 undo/redo

> **ID:** `x005b` · **slug:** `slot-session-undo` · **status:** `planned` · **area:** `[sim, host, client]` · **depends_on:** `[x005a]`  
> **ladder:** Wave 2 · [inventory-ladder.md](../inventory-ladder.md)

目標: Resonite 相当の **手元コンテキストメニュー undo/redo** を保証する。空間操作（例: 物体を A→B へ移動／渡す／置く）の直後、手元から undo すると **B→A** に戻り、redo すると **A→B** に再適用される。永続履歴・VCS ではない。

## 参照（Resonite）

| 操作 | 結果 |
|:---|:---|
| 物体を A → B に移したあと、手元コンテキストで **undo** | B → A（直前の空間遷移を逆変換） |
| 続けて **redo** | A → B（同じ遷移を再適用） |

Alchemy でも「誰の手元メニューか」＝操作者のローカル UI から、**自分が起こした直近の Slot／親子遷移**を巻き戻す。

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
- **スコープ**: まず「親子／Slot の遷移」1 種類。TRS のみの微調整 undo は必要なら後続 Slice
- **主体**: undo スタックは操作者（sub / slot）単位。他人の pass を自分のメニューから消さない
- **入口**: system_ui の個人コンテキスト（Dash 外のワールド操作後に手元から）を想定。詳細バインドは実装時に固定

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice（Resonite 相当の意味を固定） | planned |
| 1 | 遷移レコード + undo/redo + tests（A→B / B→A） | planned |
| 2 | 手元コンテキスト入口（最小）+ host 配線 | planned |

## 完了条件

1. A→B（pass または place/reparent）→ undo で A 側に戻るテスト
2. 続けて redo で再び B 側になるテスト
3. 退出でスタック破棄（プロセス／部屋終了）を文書化
4. 他ユーザの遷移が自分の undo 対象に混ざらない（最小）

## やらない

- Prefab 保存の undo
- Origin Cert
- Resonite 全コンテキスト項目の移植

← [目次](../README.md) · [ladder](../inventory-ladder.md)