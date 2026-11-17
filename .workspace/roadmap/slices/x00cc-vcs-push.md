# x00cc — push API（negotiate / push / ブランチ保護）

> **ID:** `x00cc` · **slug:** `vcs-push` · **status:** `done` · **area:** `[assets, protocol, docs]` · **depends_on:** `[x00cb, x00ca, x00c9, x00b4]`  
> **優先度:** [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §18.5  
> **後続:** [x00cd](x00cd-vcs-http-store.md) HTTP + vcs-store（§18.6）；fork / PR（§18.8–9）

目標: Elixir の `negotiate` / `push` でオーナーが新 Tree/Commit をサーバに受理できるようにする。`vcs_refs.protected` とブランチ保護切替を入れ、1 TX で objects / repo_commits / repo_blobs / origins / Authorship / ref を確定する。

根拠: [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §5.4·§7.1·§8·§11·§18.5 · [x00cb](x00cb-vcs-visibility-reporead.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `negotiate/3` が不足 `vcs_objects` / CasObject を返す | HTTP negotiate/push；client `vcs-store` |
| `push/3` はオーナーのみ；Port verify + hash 一致 | 非オーナー push（fork 後続） |
| tip 到達可能 Commit のみ受理；到達不能 Tree は格納しない | ペイロード内の孤児オブジェクト保管 |
| 新コミット `author_sub` == push 者 | クライアント署名コミット |
| Tree 子は `:owned` Grant + CAS（prepare + TX 内再検証；解決 fail-closed） | `:imported` / `:purchased` 拒否 |
| ref CAS（old 一致）+ FF（generation 打ち切り） | non-FF / 3-way |
| `protected` ブランチへの直接 push / `advance_branch` → `:ref_protected` | ブランチ削除 API |
| joining（repo 未索引の tip 祖先）≤256、オブジェクト ≤1 MiB、object_hashes ≤512 | 未ログイン |
| joining ごとに origins + `vcs_repo_blobs`；新コミットは Authorship（TX 外 prepare → TX 内 insert） | Release on push |
| ref 未存在かつ `old_commit_hash` nil なら branch create-on-push | タグ作成 via push |

---

## 設計メモ

- ファサード: `Assets.Vcs.negotiate/3` / `push/3` / `set_branch_protection/4`；検証は `Assets.Vcs.Push`
- blob 本体は既存 `Blobs.upload`；push は VCS オブジェクトのみ受け取る
- Port `verify` 要約に `created_at_ms` / `message` を追加（Commit 行への転写用）
- Elixir API のみ（x00cb と同型）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice 文書 | done |
| 1 | Migration `protected` + Ref + set_branch_protection | done |
| 2 | Port verify 拡張 + negotiate/push | done |
| 3 | テスト + docs | done |

---

## 完了条件

1. promote 済み repo へ push → tip / objects / repo_blobs / Authorship 更新
2. negotiate が missing を正しく返す
3. protected / 非オーナー / author 不一致 / Grant 不足 / CAS mismatch / non-FF で拒否・DB 不変
4. Slice / README / draft §18 / assets-server.md 鮮度一致

## やらない

- HTTP VCS ルート
- client `vcs-store` / Save UI
- fork / PR / `:purchased`
- タグ書き換え・ブランチ削除

← [目次](../README.md) · [x00cb](x00cb-vcs-visibility-reporead.md) · [assets-vcs-draft](../../0_doc/architecture/assets-vcs-draft.md)
