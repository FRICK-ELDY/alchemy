# x0086 — assets test hermetic（reaper/GC off + CAS tmp_dir）

> **ID:** `x0086` · **slug:** `assets-test-hermetic` · **status:** `done` · **area:** `[assets, docs]` · **depends_on:** `[x0074, x001e]`  
> **優先度:** 評価 P1-3（既定並列 `mix test` のフレーク除去）

目標: assets-server の ExUnit を **hermetic** にする。test では ReservedReaper / OrphanGc を起動せず、CAS 書き込みは **`use …, cas: true`**（内部で ExUnit `tmp_dir`）の隔離パスに閉じ、共有 `priv/cas_test` の `rm_rf` 競合と SQLite busy を抑える。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P1-3** · [specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/specific-weaknesses-2026-11-05.md)（既定並列フレーク）

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `MIX_ENV=test` では reaper / orphan GC GenServer を **起動しない**（`*_start: false`）。本番・dev は既定起動 | test 中にタイマー GC が動くこと |
| CAS を触る suite は `use …, async: false, cas: true` + `Assets.CasTmp` が `cas_path` を一時ディレクトリへ向ける。`cas: true` × `async: true` は **コンパイル時**に拒否 | 全 async: true 化（SQLite Sandbox 制約は残る） |
| `run_now/0` による明示 GC / reaper 単体テストは継続 | 並列フレーク 0 の数学的証明（再発時は追加隔離） |
| x001e: boot `cas_path` は `priv/cas_test`（`TestEnvIsolationTest`・`async: false` が正本）。runtime が test で CAS env を無視 | Windows Docker bind mount 固有の全問題 |

---

## 設計（要点）

| 軸 | 内容 |
|:---|:---|
| `quota_reserved_reaper_start` / `orphan_cas_gc_start` | config 既定 `true`；`config/test.exs` で `false`。`Application` が子に載せない |
| CAS パス | boot `cas_path` の **SSoT は `config/test.exs`**（`CasTmp.boot_cas_path/0` は assertion 用ミラー）。CAS suite は `cas: true` → tmp_dir（`put_env` は VM グローバルのため **async: false 必須**） |
| 単体 | `ReservedReaper.run_now` / `OrphanGc.run_now` は GenServer 不要のまま |
| 回帰 | `CasTmpTest` が tmp 向き・boot 非書き込みを正向検証 |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / backlog / improvement | **done** |
| 1 | start flags + Application 条件付き children | **done** |
| 2 | DataCase / ConnCase + `cas: true` | **done** |
| 3 | isolation / hermetic テスト + docs | **done** |

---

## 完了条件

1. test で `Process.whereis(Assets.Quota.ReservedReaper)` / `OrphanGc` が `nil`
2. CAS 利用テストが共有 `priv/cas_test` を `rm_rf` しない（`cas: true` / tmp_dir のみ；正向テストあり）
3. `mix test`（既定並列）が安定して 0 failures
4. architecture 保証表・improvement P1-3 を更新

## やらない（本 Slice）

- ~~WAN host E2E（P1-4）~~ → [x0087](x0087-wan-host-e2e.md)
- ~~Allocation throughput（P1-5）~~ → [x0088](x0088-allocation-subject-throughput.md)
- assets 全体の async: true 化
- GPU / client 変更

## 関連

- [x0074-reserved-reaper-orphan-cas-gc.md](x0074-reserved-reaper-orphan-cas-gc.md)
- [x001e-p0-trust-boundary.md](x001e-p0-trust-boundary.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
