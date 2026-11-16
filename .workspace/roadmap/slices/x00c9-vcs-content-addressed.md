# x00c9 — Spatial VCS 内容アドレス DB + promote + Port

> **ID:** `x00c9` · **slug:** `vcs-content-addressed` · **status:** `done` · **area:** `[assets, protocol, docs]` · **depends_on:** `[x00c8, x0062, x0064, x0096, x0075, x008d]`  
> **優先度:** [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §18.2  
> **後続:** [x00ca](x00ca-origin-cert-v2-origins.md) Origin Cert v2 + 原産地台帳（§18.3）；visibility / push（§18.4+）

目標: assets-server に `vcs-object` Port を配線し、VCS を内容アドレス（Tree/Commit blake3）へ移行する。`promote_prefab` が正規 CBOR を書き、ItemRef / branch / Cert v1 が新 ID で動く。

根拠: [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §15·§17·§18.2 · [x00c8](x00c8-vcs-object-codec.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| promote 後 `commit_hash` = `blake3(正規 Commit バイト)`（Port verify 一致） | Origin Cert v2 / 原産地台帳 |
| 同一論理 Prefab tree → 同一 Tree hash | visibility / RepoRead / fork / PR / push |
| `main` branch を promote 時に作成 | client `vcs-store` / Save UI |
| ItemRef `repo@main` / `repo@commit_hash` + 子 Grant 必須 | `vcs_repo` take_out |
| Cert v1 が `commit_hash` + Tree object hash で issue/verify | 既存 UUID cert の継続検証（移行時 DELETE） |
| Prefab.Payload の `local_id` 一意 | ワールド拡張 |

---

## 設計メモ

- Port: Blake3 同型の `Assets.VcsObject` プール。JSON = `vcs_object_port`
- DB: `vcs_objects` / `vcs_commits(commit_hash)` / `vcs_commit_parents` / `vcs_repo_commits`；`tree_json` 廃止
- 移行: **破壊的**スキーマ再作成（UUID/`tree_json` 行は破棄）+ 旧 cert DELETE。本番に UUID VCS は未出荷前提。`MigrateContentAddressed.run/0` は意図的 no-op（draft §17 の in-place 再エンコードは本 Slice では実装しない）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice 文書 | done |
| 1 | Port プール | done |
| 2 | DB + promote + ItemRef + Cert | done |
| 3 | テスト + docs | done |

## 完了条件

1. promote → commit_hash が Port 再 verify と一致
2. ItemRef / FF / Cert v1 が新 ID で通る
3. `/ready` が vcs-object pool を見る；Dockerfile に bin
4. 本 Slice / README / draft 鮮度一致

## やらない

- Cert v2、push、fork、market

← [目次](../README.md) · [x00c8](x00c8-vcs-object-codec.md) · [assets-vcs-draft](../../0_doc/architecture/assets-vcs-draft.md)
