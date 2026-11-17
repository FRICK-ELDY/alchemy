# x00ca — Origin Cert v2 + 原産地台帳

> **ID:** `x00ca` · **slug:** `origin-cert-v2-origins` · **status:** `done` · **area:** `[assets, docs]` · **depends_on:** `[x00c9, x0096, x0065]`  
> **優先度:** [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §18.3  
> **後続:** [x00cb](x00cb-vcs-visibility-reporead.md) visibility / RepoRead（§18.4）；push（§18.5+）

目標: Origin Cert を v2（Authorship / Release、`issued_at_ms`・`kid`）へ上げ、blob 先願台帳 `vcs_blob_origins` を追加する。`promote_prefab` と `create_tag` の確定 TX で自動発行・台帳書き込みする。

根拠: [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §11·§11.5·§15·§17·§18.3 · [x00c9](x00c9-vcs-content-addressed.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Authorship メッセージ `v2\|commit\|{commit_hash}\|{author_sub}\|{issued_at_ms}\|{kid}` | v1 発行・v1 検証互換 |
| Release メッセージ `v2\|release\|{repo_id}\|{tag_name}\|{commit_hash}\|{signer_sub}\|{issued_at_ms}\|{kid}` | CA チェーン・失効・複数署名・JWKS HTTP |
| `issued_at_ms` と `kid` が署名対象に含まれる | 鍵ローテ自動化・`kid`→公開鍵ルックアップ |
| Authorship は `commit_hash` あたり 1 行（再 promote / 再 issue は既存を返す）。subject = `Commit.author_sub` | 偽 subject での発行 |
| 既存行と subject / `target_id` が食い違う再発行は `:cert_conflict` | — |
| Release は `(repo_id, tag_name)` あたり 1 行。subject = owner・tag の commit 一致必須 | `create_tag` への owner_sub 引数追加 |
| 署名は TX 外（既存あればスキップ）；TX 内 `insert_prepared_*` で束縛再検証＋insert | 販売ロック拒否・market 出品照合 |
| origins は先願 1 行（ON CONFLICT DO NOTHING）。`blake3` は changeset で小文字 64-hex 正規化 | 知覚ハッシュ・改変コピー検出 |
| オフライン検証はメッセージ＋署名＋**呼び出し側が渡す公開鍵**（省略時は現行鍵） | HTTP export / verifier 着用監査 |
| 秘密鍵はログ・レスポンスに出ない | License Token / pgvector |

---

## 設計メモ

- v1 は x00c9 で DELETE 済み・本番未出荷前提のため **廃止**（再発行パスなし）
- `kid` = `Application.get_env(:assets, :cert_signing_kid, "default")`。メッセージに載るが検証は kid 解決しない（現行鍵 or 明示 pub key）
- Authorship: `commit_hash` が Tree/親/作者を内容アドレスで束ねる（v1 の tree/parent フィールドは不要）
- 発行シーケンス: TX 外 `prepare_*_unless_present` → TX 内 `insert_prepared_*`（束縛を両側で fail-closed）
- DB: partial unique `vcs_certs_authorship_commit_index` / `vcs_certs_release_repo_tag_index`
- 原産地台帳: PK `blake3`（`BlobOrigin.changeset` が downcase + `^[0-9a-f]{64}$`）；promote 時は Prefab.Payload の子 mesh blake3 のみ
- ブランチ advance には Cert を出さない（§11.3）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice 文書 | done |
| 1 | Migration（`vcs_certs` v2 列 + `vcs_blob_origins`） | done |
| 2 | `Assets.Vcs.Cert` v2 + BlobOrigin | done |
| 3 | promote / create_tag 配線 | done |
| 4 | テスト + docs 鮮度 | done |

---

## 完了条件

1. promote → Authorship cert 1 行 + 子 blake3 の origins（重複 promote で先願維持）
2. create_tag → Release cert；メッセージに repo/tag/commit/owner/`issued_at_ms`/`kid`
3. 署名・`target_hash`・フィールド改ざんは `:tampered`；オフライン検証 OK
4. 本 Slice / README / draft §18 / assets-server.md 鮮度一致

## やらない

- visibility / push / fork / PR
- 販売ロック・`:purchased` / market
- client `vcs-store` / HTTP Cert API

← [目次](../README.md) · [x00c9](x00c9-vcs-content-addressed.md) · [assets-vcs-draft](../../0_doc/architecture/assets-vcs-draft.md)
