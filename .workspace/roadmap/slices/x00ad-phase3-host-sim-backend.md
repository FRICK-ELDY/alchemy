# x00ad — Phase 3 着手: host `SimBackend` + 接触オン

> **ID:** `x00ad` · **slug:** `phase3-host-sim-backend` · **status:** `done` · **area:** `[host, sim, client, docs]` · **depends_on:** `[x00a8, x00ac]`  
> **優先度:** [mvcc_xpbd.md](../mvcc_xpbd.md) Phase 3 着手 / improvement-plan **P1-2（host 配線）**  
> **後続:** [x00ae](x00ae-phase3-dirty-delta-codec.md) dirty codec **done** · PROTOCOL_PIN · `…/sim_state` · `…/edit`（次空き `x00af`〜）

目標: Phase 2（x00a9–x00ac）Green 後、`host-server` 権威ループに **切替可能な `SimBackend`** を入れ、**body-body 接触をオン**（`xpbd::step(..., bounds: Some)`）にして improvement-plan **P1-2 の host 側**を閉じる。x0093 で意図的に切っていた LBVH 接触を、不等式経路（x00a8）のまま本線に戻す。

根拠: [mvcc_xpbd.md](../mvcc_xpbd.md) §0 / §5 Phase 3 · [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) P1-2 · [x0093](x0093-xpbd-wire-authority.md)（接触 OFF の記録）· [x00a8](x00a8-inequality-contact.md)

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| Phase 2 DoD（ArcSwap / jitter / 手元予測 / `interp.wgsl`） | **done**（x00a9–x00ac） |
| 不等式接触 + ページ経路接触回帰 | **done**（x00a4 / x00a8；host 未配線） |
| Room AEAD（x0069）·wire_cap を **バイパスしない**（既存 frame / snapshot 経路を維持） | 本スライスで **契約維持**を明示 |
| handover は §4.8（Q1 採用済み） | **本スライスでは実装しない**（→ 後続 `sim_state`） |

破ってはならないこと: 独自トランスポート、検証スキップ、受信者別 `V_ack`、等式距離のまま「接触オン」、デフォルトを壊して smoke を赤にする無ガード切替。

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `ALCHEMY_SIM_BACKEND=epoch_xpbd\|mvcc_xpbd`（既定 `epoch_xpbd`）で権威ステップを切替 | dirty デルタ / `WireDirtyAccumulator` / Smallest-Three |
| **接触オン**: 選択中バックエンドの `step` が `bounds: Some(SceneBounds)`（LBVH + `FLAG_INEQUALITY`） | 箱 vs 箱の正確 OBB；外接球近似の撤廃 |
| host 経路でも 2 動体が重なったまま放置されない（P1-2 host） | PROTOCOL_PIN 追加（`sim_tick` / `holder` / `acked_intent_seq` 等） |
| 既存 **地面平面**・**Grab begin/follow/release**・Intent loco が `epoch_xpbd` で退行しない | 計画的 / crash `…/sim_state`（§4.8） |
| `mvcc_xpbd`: `PageXpbdAdapter`（または同等）でページ上 XPBD＋接触；Ingress 入力は `is_finite` 検疫後に書く | 本番デフォルトを `mvcc_xpbd` に即切替；フルメッシュ解除 |
| frame / snapshot の Room AEAD・wire_cap ACL を迂回しない | 確定レーン `…/edit`；Spatial Interest |
| `#![forbid(unsafe_code)]`（shared-sim / host-server）維持 | 拘束ソルバー全体の SIMD；規模 CCU |

---

## 実装メモ

* `client/host-server/src/sim_backend.rs` — `SimBackendKind` + env パース（未知値は Err）
* `PhysicsWorld::from_entries_with_backend` → `Result`（容量超過も起動失敗）/ `step_dt` — `epoch_xpbd` は `bounds: Some`；`mvcc_xpbd` は sheet→page→`step_page_with_grabs`→sheet（一時ブリッジ）
* page `holder_slot` は host Grab 実スロットを同期（slot 0 固定禁止）
* 接触ペア: DYNAMIC または static BOX。user_root センサ球は非参加（`pull_page` は `inv_mass>0` のときだけ DYNAMIC — 全行 DYNAMIC 禁止）
* `PageXpbdAdapter::step_page_with_grabs` / `set_gravity` / `set_substeps`（shared-sim）
* 回帰: `host_body_body_*` · `mvcc_backend_*` · `mvcc_ground_*` · `mvcc_grab_*` · `mvcc_page_holder_*`

### レビュー取り込み（軽微）

* `try_ingest_mvcc_kinematic_for_test` は `#[cfg(test)]` のみ（本番 Ingress ではない）
* `mvcc_xpbd` では `apply_frame_delta_to_prev` を呼ばない（page pull が Prev 再構築）
* cell_origin=0 近傍限定・sheet↔page 往復 zero-alloc 未達は非保証に明記

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap 目次 + ゲート確認 | **done** |
| 1 | `SimBackend` 抽象 + env 切替 | **done** |
| 2 | `epoch_xpbd` で `bounds: Some` + 地面 / Grab / 2 箱 host 回帰 | **done** |
| 3 | `mvcc_xpbd` 経路（adapter step + 検疫）+ 保証表更新 | **done** |

---

## 完了条件

1. `ALCHEMY_SIM_BACKEND` 未設定時は `epoch_xpbd`；不正値は起動失敗（黙って別 backend にしない）— **done**
2. `epoch_xpbd` で接触オン: 2 動体が貫通したまま残らない — **done**（`host_body_body_contacts_separate_overlapping_boxes`）
3. 回帰: 地面・Grab・Intent — **done**（既存 host テスト Green）
4. `mvcc_xpbd`: `step_page`（接触付き）+ 非有限拒否 — **done**
5. `cargo test -p alchemy-shared-sim` / `cargo test -p host-server --bin host-server` Green — **done**
6. 本スライスで **dirty / `sim_state` / PROTOCOL_PIN / `…/edit` を入れない** — **done**
7. roadmap 目次と本ファイル `status: done` — **done**

---

## 起動・検証メモ

```powershell
cargo test -p host-server --bin host-server
cargo test -p alchemy-shared-sim --lib

$env:ALCHEMY_SIM_BACKEND = "mvcc_xpbd"
cargo test -p host-server --bin host-server mvcc_backend
```

ブランチ: `feat/x00ad-phase3-host-sim-backend`
