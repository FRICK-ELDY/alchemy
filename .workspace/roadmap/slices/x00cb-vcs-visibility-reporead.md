# x00cb — visibility / RepoRead / ItemRef / private 404

> **ID:** `x00cb` · **slug:** `vcs-visibility-reporead` · **status:** `done` · **area:** `[assets, docs]` · **depends_on:** `[x00ca, x00c9, x0063, x00b4]`  
> **優先度:** [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §18.4  
> **後続:** push API（§18.5）；fork / PR / public→private 切り離し（§18.8–9）

目標: `vcs_repos.visibility` と `vcs_repo_blobs` を導入し、RepoRead を blob GET 認可に足す。`ItemRef` の `repo_rev` を「オーナーか」から「読めるか」（§9.1）へ変え、private 非オーナーは 404 で存在を漏らさない。

根拠: [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §9.1–9.2·§15·§17·§18.4 · [x00ca](x00ca-origin-cert-v2-origins.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `visibility` は `private` \| `public`。promote 既定は `private` | `default_branch` / fork 列 / ブランチ保護 |
| オーナーは `set_visibility/3` で切替可 | HTTP VCS API；UI 警告文 |
| promote で Prefab 子 mesh blake3 を `vcs_repo_blobs` に索引 | push / fork / マージ時の索引更新（後続） |
| RepoRead: 読める repo の `vcs_repo_blobs` に含まれる blake3 は GET 可（Grant / PlaceRead に加えて） | 未ログイン public 閲覧（§19 未決） |
| public だからといって Grant なし・索引なしで返すことはしない | CAS 不在時の区別緩和 |
| `ItemRef` `repo_rev`: `can_read_repo?` + 子 ∈ `vcs_repo_blobs`（個人 BlobGrant は不要） | `vcs_repo` take_out；spawn 実行 |
| 索引欠落は `{:error, :incomplete_index}`（`:no_grant` と分離） | HTTP マッピング（未配線） |
| private 非オーナーの repo 読み取り（ItemRef / `fetch_readable_repo`）は `:not_found`（404） | blob GET 拒否を 404 に統一（既存どおり `:forbidden`）；`set_visibility` 非オーナーは当面 `:forbidden` |
| public→private 後、他人の ItemRef は 404 | fork 切り離し・PR クローズ（fork 未実装） |
| private→public は許可 | `:purchased` 再配布チェック（market 未導入） |

---

## 設計メモ

- 認可の単一入口: `Assets.RepoRead`（`can_read_repo?` / `fetch_readable_repo` / `allows_blake3?` / `assert_repo_blobs`）
- blob GET: `Blobs.authorize_and_fetch` に RepoRead を第 3 条件として追加（既存 Grant・PlaceRead は維持）。`allows_blake3?` は `Repo.exists?/1`（キャッシュなし）
- ItemRef: `fetch_repo_as_owner` → `RepoRead.fetch_readable_repo`；子 Grant は `assert_repo_blobs` に置換（x00b4 の **保存**経路の子 Grant は変更しない）
- Elixir API のみ（promote / ItemRef と同様 HTTP 未配線）
- blob GET の拒否は従来どおり `:forbidden`（CAS 存在は漏らすが、repo 存在とは別軸）
- **マイグレーション**: `vcs_repo_blobs` のバックフィルなし。x00c9 同様 hermetic / 未出荷前提 — 既存 promote 行があるローカル DB は **捨てる or 再 promote**。本番 in-place バックフィルは積まない
- `set_visibility` の非オーナー `:forbidden` は読取経路の `:not_found` と非対称。HTTP 化時に揃える

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice 文書 | done |
| 1 | Migration（visibility + `vcs_repo_blobs`） | done |
| 2 | `Assets.RepoRead` + promote 索引 + `set_visibility` | done |
| 3 | ItemRef / Blobs.fetch 配線 | done |
| 4 | テスト + docs 鮮度 | done |

---

## 完了条件

1. promote → `visibility=private` + 子 blake3 が `vcs_repo_blobs` に入る
2. owner が public にした repo: 他ユーザーが ItemRef 解決でき、Grant なしでも GET が RepoRead で通る
3. private のまま / public→private 後: 他ユーザー ItemRef は `:not_found`
4. 本 Slice / README / draft §18 / assets-server.md 鮮度一致

## やらない

- push / negotiate / ブランチ保護
- fork / `:imported` / PR / upstream_detached
- client `vcs-store` / HTTP VCS ルート
- 未ログイン public 閲覧
- blob GET 拒否の 404 統一

← [目次](../README.md) · [x00ca](x00ca-origin-cert-v2-origins.md) · [assets-vcs-draft](../../0_doc/architecture/assets-vcs-draft.md)
