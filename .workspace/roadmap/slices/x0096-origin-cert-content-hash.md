# x0096 — Origin Cert に tree/content hash（v1 ペイロード固定）

> **ID:** `x0096` · **slug:** `origin-cert-content-hash` · **status:** `done` · **area:** `[assets, docs]` · **depends_on:** `[x0065]`  
> **優先度:** improvement **P2-4**（連合前にペイロード形を固定）

目標: Origin Cert の署名対象を `target_kind:target_id:subject_sub` だけから、**内容ハッシュ（Commit `tree_json` の BLAKE3）と親 commit** を含む **v1** 形式へ固定する。DB 外（連合・マーケット）で「どの内容の作者か」を示せる形にする。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-4** · [opus-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-05.md) · [opus-specific-proposals-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-proposals-2026-11-05.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 署名メッセージは `v1\|{kind}\|{id}\|{sub}\|{tree_blake3}\|{parent}`（`\|` 区切り・version 接頭） | 旧 `kind:id:sub` 形式との互換検証 |
| `tree_blake3` = BLAKE3 hex of Commit.`tree_json`；Tag は tagged Commit 経由 | Merkle of child blob blake3（tree_json 全体ハッシュで足りる） |
| `parent` = `parent_commit_id` または空（root） | CA chain / 失効 / License Token / 第三者 CA |
| issue: 未知 Commit/Tag → `:not_found`；`subject_sub` に `\|` → `:invalid_subject_sub` | length-prefix 符号化（パイプ + sub 文字種制約で代替） |
| verify: cert 行無し → `:not_found`；tree/parent/署名不一致 **または対象消失** → `:tampered`（fail-closed） | GC と改ざんのエラー区別 |
| 秘密鍵はレスポンス・ログに出ない（x0065 維持） | オフライン export スキーマの HTTP API |

---

## 設計（要点）

```
issue(commit|tag)
  assert subject_sub has no "|"
  resolve → Commit
  tree_blake3 = blake3(tree_json)
  parent     = parent_commit_id || ""
  message    = "v1|…|tree_blake3|parent"
  sign(message) ; target_hash = blake3(message)
```

- スキーマ列は変更しない（`target_hash` は引き続き message の blake3 アンカー）
- ペイロード形の破壊的変更は連合前の今に閉じる（thin peel 時点で固定）
- **移行:** 旧 `kind:id:sub` で発行済みの行は v1 では verify 不能 → **既存行は無効。再発行が必要**
- verify で対象 Commit/Tag が消えている場合も `:tampered`（cert はあるが内容束縛を証明できない。issue の `:not_found` との非対称は意図的）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / improvement | **done** |
| 1 | `Assets.Vcs.Cert` v1 message + resolve Commit/Tag | **done** |
| 2 | 否定系テスト（tree / parent 改ざん・対象消失・未知 tag）+ architecture 注記 | **done** |

---

## 完了条件

1. 正規 Cert 検証 OK；signature / target_hash / tree_json / parent 改ざん・対象消失は `:tampered`
2. canonical message に `v1`・`tree_blake3`・`parent` が含まれる
3. improvement P2-4 を done 表記

## やらない（本 Slice）

- verifier 着用監査 / License Token
- vcs_certs 列追加（content_hash の denormalize）
- pgvector / 先願ロック DB 拒否
- 旧形式の自動再署名マイグレーション

## 関連

- [x0065-origin-cert-thin.md](x0065-origin-cert-thin.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
