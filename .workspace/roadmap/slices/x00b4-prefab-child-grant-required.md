# x00b4 — 保存時の子 Grant 必須化（Prefab / Vcs ツリーの全入口）

> **ID:** `x00b4` · **slug:** `prefab-child-grant-required` · **status:** `done` · **area:** `[assets, docs]` · **depends_on:** `[x0060, x0062, x0063, x007f]`  
> **優先度:** [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §14.4 対策 2（VCS 本体より先行・assets のみで閉じる）  
> **後続:** [x00b5](x00b5-spawn-ticket-ownership-binding.md)（配置時の所有権バインド）

目標: Prefab / Vcs ツリーの子 blob を参照するすべての入口で、オーナーが子ごとに BlobGrant を持つことを要求する。「部屋で見かけた他人の hash を自分の Drive やリポジトリに保存し、後の経路で読み取り権に化けさせる」余地を閉じる。

根拠: [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §14.4 · [assets-threat-model.md](../../0_doc/architecture/assets-threat-model.md) 黄金規則 1（hash を知っている＝所有ではない）· `Assets.Drive` moduledoc の「意図的な穴」（save 時は子 Grant 不要、take_out 側で拒否）

---

## 着手ゲート（満たしていること）

| 条件 | 状態 |
|:---|:---|
| take_out の子 Grant 検証（x007f） | **done** |
| HTTP put_back（x006e） | **done** |
| Vcs promote / ItemRef repo@rev（x0062 / x0063） | **done**（HTTP 未配線） |

破ってはならないこと: take_out 側の子 Grant 検証（多層防御として残す）、Payload スキーマ（x005e）、save→reload の可逆性（x0061）、Quota の予約→確定の流れ（x003f）。

---

## 入口（実装後）

| 入口 | 子 Grant の確認 | 備考 |
|:---|:---|:---|
| `Drive.save_prefab/4`（HTTP `POST /api/v1/drive/put_back`） | **あり**（upload 前） | 不足 → `:no_grant` / HTTP 403；Quota・CAS 非消費 |
| `Drive.create_prefab_node/4` | **あり**（payload Grant → CAS → 子） | 不正 payload → `:invalid_prefab`；HTTP 未公開 |
| `Vcs.promote_prefab/2` | **あり**（昇格 TX 内） | 不足 → rollback `:no_grant`；HTTP 未公開 |
| `ItemRef.resolve/2`（`repo_rev`） | **あり**（ツリー decode 後） | 不足 → `:no_grant`；HTTP 未公開 |
| `Drive.resolve_for_take_out/2` | あり（x007f） | 多層防御として維持 |

---

## 保証する / しない（本スライス）

| 保証する | 保証しない |
|:---|:---|
| `save_prefab/4`: 子すべてに Grant が無ければ、アップロード前に `:no_grant`（Quota を消費しない） | 既存の DriveNode / Commit の遡及検査・削除 |
| `create_prefab_node/4`: payload を CAS から読んで検証し、子すべての Grant を要求する（不正な payload は `:invalid_prefab`） | Grant の失効・取り消し API |
| `promote_prefab/2`: 昇格時に子すべての Grant を再検証する（不足は TX ロールバックで `:no_grant`） | VcsRepo の take_out（引き続き非保証） |
| `ItemRef.resolve/2`（`repo_rev`）: 子すべての Grant を確認する（不足は `:no_grant`） | public リポジトリの共有・RepoRead（assets-vcs-draft の後続） |
| HTTP `put_back`: 子 Grant 不足は 403 `no_grant` | client の put_back 失敗表示の改善（既存のエラー表示のまま） |
| take_out 側の子 Grant 検証（x007f）を維持する | 配置時の所有権バインド（x00b5） |

---

## 実装メモ

* 子 Grant の判定は 1 か所に寄せ、Drive / Vcs / ItemRef から同じ関数を使う（現状は `Assets.Drive` の private 関数）
* `save_prefab`: `maybe_assert_parent_owner` の後、`Blobs.upload` の前に判定する（拒否時に Quota と CAS を消費しない）
* `create_prefab_node`: payload の Grant → CAS 読み出し → `Payload.from_json` → 子 Grant の順
* `promote_prefab`: `load_prefab_json!` の後に decode と子 Grant を確認し、失敗は `Repo.rollback`
* `ItemRef`（`repo_rev`）: コミットのツリーを decode した後に子 Grant を確認する
* `PROTOCOL_PIN`: 変更なし（assets の HTTP 内部挙動のみ）
* client: put_back の 403 は既存の `put_back status=… body=…` エラーとして表示される（`client/app/src/inventory.rs`）。変更不要

### テストの扱い（削除・無効化はしない）

* ダミー hash（`"aaaa…"` など）を子に入れて `save_prefab` / `put_back` を成功させている既存テストは、子を実際に `Blobs.upload` してから使う形に直す（保証の変更に伴う前提の修正）。対象: `drive_take_out_test.exs` / `drive_test.exs` / `drive_controller_test.exs` / `vcs_test.exs`
* `drive_test.exs` の「保存は成功し、take_out で `:no_grant`」は 2 本に分ける:
  1. 子 Grant が無いと保存時に `:no_grant`
  2. 保存後に子の Grant を消すと take_out で `:no_grant`（take_out 側の検証が残っていることの確認）
* 新規: `save_prefab` / `put_back`（403）/ `create_prefab_node` / `promote_prefab` / `ItemRef` `repo_rev` の子 Grant 不足、および拒否時に Quota が増えないこと

### 文書の更新

* `Assets.Drive` moduledoc: 「意図的な穴」と Non-guarantees の該当行を削除し、Guarantees に移す
* `assets.ex` moduledoc の Guarantees
* [assets-server.md](../../0_doc/architecture/assets-server.md) 保証表（「`save_prefab` 時の子 Grant 必須化（take_out 側で弾く）」を保証側へ）
* [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §14.4 の現状列

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap + assets-vcs-draft §14.4 | **done** |
| 1 | 子 Grant 判定の共通化 + `save_prefab` / HTTP `put_back` | **done** |
| 2 | `create_prefab_node` / `promote_prefab` / `ItemRef` `repo_rev` | **done** |
| 3 | テストの前提修正・新規テスト・moduledoc / assets-server.md | **done** |

---

## 完了条件

1. 上の 4 入口すべてで、子 Grant が 1 つでも欠けると拒否される
2. `save_prefab` の拒否時に Quota（`used_bytes` / `reserved_bytes`）と CAS が増えない
3. take_out 側の子 Grant 検証が残っている（Grant を消した後の take_out が `:no_grant`）
4. 既存テストを削除・無効化せずに Green（前提の修正のみ）
5. assets-server.md / moduledoc の保証表がコードと一致する

---

## 起動・検証メモ

```powershell
cd assets-server
mix format
mix test test/assets/drive_test.exs test/assets/drive_take_out_test.exs test/assets/drive_node_test.exs test/assets/vcs_test.exs test/assets_web/controllers/drive_controller_test.exs
mix test
```

推奨ブランチ: `feat/x00b4-prefab-child-grant-required`

← [目次](../README.md)
