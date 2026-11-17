# x00d0 — dash / desktop イベント→コマンド分割（P2-1）

> **ID:** `x00d0` · **slug:** `desktop-event-command-split` · **status:** `done` · **area:** `[client, docs]` · **surface:** `[desktop]` · **depends_on:** `[x0053, x007c, x00ce]`  
> **優先度:** improvement-plan **P2-1**（負債・衛生）

目標: Desktop の `UiAction` **実行**（session / inventory / hotswitch）を UI イベントハンドラから分離し、`surface/desktop/{session,inventory,hotswitch}_flow` に置く。`system_ui` はコマンド enum を出すだけ（Facet 契約維持）。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) **P2-1** · [opus-specific-weaknesses-2026-11-12.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-12.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `system_ui` は `UiAction` を出し、HTTP / Zenoh / host spawn を持たない | Dash レイアウトの行数削減（`dash.rs` は本 Slice 対象外） |
| session / inventory / hotswitch オーケストレーションは `surface/desktop/*_flow` | 新コマンド enum の導入（既存 `UiAction` を維持） |
| 共有ジョブバス（`AuthJobOutcome`）は `desktop/job.rs` | world-server Rooms / FallbackController 分割（P2-8） |
| 挙動は分割前と同等（リファクタのみ） | winit E2E の新規追加 |

---

## 設計

```text
system_ui on_click → UiAction
        ↓
desktop/mod.rs  handle_ui_action（薄い match）
        ├─ session_flow   Auth* / Start/Join / Worlds* / host / leave / endpoints / mTLS reopen
        ├─ inventory_flow Inventory* / put_back / take_out / RoomSave
        ├─ hotswitch_flow auto / F8 / presentation leave·exit
        ├─ job            AuthJobOutcome + worker spawn（葉: support のみ依存）
        └─ support        stop_host / inventory mirror / catalog_owner（葉）
```

レビュー追従: `job`↔flow 三角循環を `support` で切断。`InventoryTakeOut*` は `inventory_flow::apply_inventory_job_outcome`。さらなる `worlds_flow` 切り出し（行数）は後続可。

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | `desktop.rs` → `desktop/mod.rs` | **done** |
| 2 | `job` / `session_flow` / `inventory_flow` / `hotswitch_flow` 抽出 | **done** |
| 3 | 単体テスト移動・追加 + docs 鮮度 | **done** |

---

## 完了条件

1. `handle_ui_action` に session/inventory/hotswitch の HTTP・host・Zenoh オーケストレーションが残っていない（ディスパッチのみ）。例外: Exit の `presentation.request_exit` + `event_loop.exit` は `ActiveEventLoop` 都合の薄い糊として `mod.rs` に残す
2. `system_ui` / `dash.rs` に新規の Room/HTTP 知識を増やしていない
3. `cargo test -p client` で既存＋新規単体が Green
4. Slice / README / improvement-plan P2-1 / client.md が一致

## やらない

- P2-8 Rooms / FallbackController（world-server）
- `UiAction` の別 enum 化
- `dash.rs` のレイアウト分割
- 認証・権限チェックの弱体化

← [目次](../README.md) · [improvement-plan-2026-11-12](../../0_doc/evaluation/improvement-plan-2026-11-12.md) · [x00cf](x00cf-refresh-os-vault.md)
