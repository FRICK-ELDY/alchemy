# x00d6 — Rooms / FallbackController 分割（P2-8）

> **ID:** `x00d6` · **slug:** `rooms-fallback-split` · **status:** `done` · **area:** `[world-server, docs]` · **depends_on:** `[]`  
> **優先度:** improvement-plan **P2-8**（負債・衛生）

目標: `WorldWeb.FallbackController` に Room API のエラー表を一箇所化し、`World.Rooms` を `Lease` / `RoomKey` / `Scene` / `Catalog` に分割する。HTTP ステータスと JSON `error` 文字列は変更しない。`World.Rooms` facade（公開 API）は維持する。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) **P2-8** · [opus-specific-weaknesses-2026-11-12.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-12.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Room HTTP の status / `error` 文字列は分割前と同等 | RelationController / assets Fallback の拡張 |
| 曖昧 domain atom（`:conflict` / `:forbidden_host` 等）は controller 境界で一意化してから Fallback へ | ドメイン層のエラー atom 改名（Rooms は従来どおり `:conflict` 等を返す） |
| `World.Rooms` 公開関数は facade 経由で到達可能（既存呼び出しは壊さない） | RoomController の複数 Phoenix controller への分割 |
| spawn claim 解放などエラー時副作用は Fallback 到達前に実行 | 新規 E2E / 認証・権限の緩和 |

---

## 設計

```text
RoomController action
  ├─ success → json(conn, …)
  ├─ side-effect on error（SpawnTicket release 等）→ then {:error, unique_atom}
  └─ {:error, unique_atom | tagged} → action_fallback
                                              ↓
                                    FallbackController.call/2
                                      （一意 atom → status + %{error: …}）

World.Rooms（facade: create / owner / visibility / ACL + defdelegate）
  ├─ Rooms.Scene    pack/unpack · scene · overlay · save · CAS
  ├─ Rooms.Lease    host session · lease · handover · takeover · generation
  ├─ Rooms.RoomKey  rotate · due rotate · hex / token key
  └─ Rooms.Catalog  list_visible_summaries
```

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | FallbackController エラー表 + RoomController 正規化 | **done** |
| 2 | Rooms.Internal / Scene / Lease / RoomKey / Catalog 抽出 + facade | **done** |
| 3 | `mix test` + docs / improvement-plan P2-8 closed | **done** |

---

## 完了条件

1. `RoomController` の同型 `else` / `render_*_error` が Fallback に集約され、重複エラー表が残っていない
2. `World.Rooms` の Scene / Lease / RoomKey / Catalog 実装が子モジュールにあり、facade が委譲する
3. `mix test`（world-server）が Green（既存 HTTP 契約テスト含む）
4. Slice / README / improvement-plan P2-8 / world-server.md が一致

## やらない

- 認証・権限・署名検証の弱体化
- HTTP エラー語彙の意図的変更
- assets-server Fallback の拡張
- RoomController のルート分割

← [目次](../README.md) · [improvement-plan-2026-11-12](../../0_doc/evaluation/improvement-plan-2026-11-12.md) · [x00d5](x00d5-keystale-seamless-rejoin.md)
