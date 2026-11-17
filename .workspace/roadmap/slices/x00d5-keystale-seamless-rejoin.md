# x00d5 — KeyStale 無停止二相切替 + XR Rejoin（P2-4）

> **ID:** `x00d5` · **slug:** `keystale-seamless-rejoin` · **status:** `done` · **area:** `[client, docs]` · **depends_on:** `[x007e, x00d2, x0053, x00b6]`  
> **優先度:** improvement-plan **P2-4**（KeyStale 無停止二相切替 + XR Rejoin）

目標: guest が epoch `+1` 前進時に Leave せず、旧鍵受信を続けながら新 Token を pending に置き、新 epoch Manifest 検証後に `room_key`→`pending` ロック順で live←pending を一括更新する（単一 Mutex 完全アトミックはしない）。grace 失敗・世代飛び（`+2` 以上）時は KeyStale → 明示 Rejoin。XR は KeyStale overlay 同期と Rejoin/Leave を PresentationGate 経由で Desktop に委譲する。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) **P2-4** · [gpt-specific-proposals-2026-11-12.md](../../0_doc/evaluation/gpt/gpt-specific-proposals-2026-11-12.md) · [x007e](x007e-room-key-epoch.md)（手動 KeyStale）· [x00d2](x00d2-catalog-rl-epoch-api.md)（軽量 epoch probe）

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| epoch 前進で guest が新 Token を `pending_room_aead` に stash；live AEAD は旧のまま受信継続 | Zenoh epoch bump push / ヒント |
| wire open は pending→live の dual-open（protocol `open_current_or_prev` 順）；seal は pending 優先 | guest Token への `room_key_prev*` 同梱；3 世代以上の受理 |
| 新 epoch Manifest の AEAD open + seal 検証成功で、`room_key`→`pending` ロック順の下に live←pending を一括更新してから pending クリア・`bus_generation` bump（Leave なし；Leave 交差は live 空／bus down で拒否） | 単一 Mutex での完全アトミック；commit 成功パスでの Zenoh subscriber 張り直し / Leave→Join |
| pending grace（≈300s・host prev grace 相当）超過時は KeyStale + remount/bus bump → 明示 Rejoin | 無停止失敗時の自動 Rejoin |
| XR: `shared.key_stale()` → overlay 同期；`WorldsRejoin` / `WorldsLeave` は PresentationGate → Desktop 実行 | XR 単独の Join/Host/catalog HTTP・Zenoh |

---

## 設計（要点）

### Guest pending + dual-open

1. Token refresh で `token_epoch == live_epoch + 1` → `PreparePending`（即 KeyStale にしない）
2. `pending_room_aead: (key, epoch, deadline)` を stash；live は旧
3. `open_zenoh_payload`: pending を current、live を prev として dual-open；戻りに開いた epoch
4. `seal_zenoh_payload`: pending があればそれで seal
5. Manifest Accept かつ opened_epoch == pending.epoch → `try_commit_pending_room_aead`（`room_key`→`pending` 順；pending クリアは live 更新後）
6. grace 超過は **同一 pending ロック内**で再確認してから clear → KeyStale（commit 成功直後の誤 KeyStale を防ぐ）
7. `token_epoch > live + 1` → 即 KeyStale（host current+prev とずれる世代飛び）
8. 同 epoch soft-TTL `Commit` は `pending.epoch > epoch` の stash を消さない

### bus generation

- `SharedState.bus_generation: AtomicU64`（`remount_generation` とは別）
- bump: pending commit / KeyStale / Leave / **epoch が変わる** Join・Rejoin。同 epoch soft-TTL `Commit` では bump **しない**
- open/seal は世代スナップショット；交差時は fail-closed（次サンプルで再試行）

### XR Rejoin

- `PresentationGate.rejoin_requested` / `session_leave_requested`
- XR `WorldsRejoin` → `request_rejoin()`；`WorldsLeave` → `request_session_leave()`
- Desktop `poll_presentation_requests` → `worlds_rejoin` / `worlds_leave`

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | pending AEAD / bus_generation / dual-open / Manifest 一括更新（ロック順） | **done** |
| 2 | XR Rejoin + KeyStale overlay 同期 | **done** |
| 3 | 保証表・improvement-plan P2-4 closed・単体 | **done** |

---

## 完了条件

1. epoch 前進で guest が Leave せず live AEAD が新 epoch に切り替わる（単体: refresh ポリシー + dual-open）。
2. commit 前は旧・新暗号文を dual-open；commit 後は旧鍵のみでは新暗号文を読めない（否定系）。
3. grace 失敗時は KeyStale overlay + Rejoin で Live 復帰。
4. XR companion で KeyStale 表示と Rejoin が Desktop 実行で完了する。
5. client.md / improvement-plan P2-4 が一致。

## やらない

- Zenoh epoch bump push
- PROTOCOL_PIN bump
- guest Token への prev 鍵同梱
- ~~P2-8 Rooms / Fallback 分割~~ → [x00d6](x00d6-rooms-fallback-split.md)
- XR 単独 HTTP Join/Host/catalog

## 関連

- [x007e-room-key-epoch.md](x007e-room-key-epoch.md)
- [x00d2-catalog-rl-epoch-api.md](x00d2-catalog-rl-epoch-api.md)
- [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md)
- [client.md](../../0_doc/architecture/client.md)

← [目次](../README.md)
