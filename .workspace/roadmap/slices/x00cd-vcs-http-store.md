# x00cd — HTTP negotiate/push + 最小 vcs-store

> **ID:** `x00cd` · **slug:** `vcs-http-store` · **status:** `done` · **area:** `[assets, client, docs]` · **depends_on:** `[x00cc]`  
> **優先度:** [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §7.1·§18.6  
> **後続:** [x00ce](x00ce-grab-radial-drive-save.md) グラブ→ラジアル Save→Drive；Git 詳細パネル；clone/fetch；fork / PR（§18.8–9）

目標: 既存 Bearer JWT の `/api/v1` に negotiate/push を載せ、client `vcs-store` で init / commit / push の最小ローカル CAS を動かす。SSH は採用しない（TLS 終端＋トークンで既存 API と同型）。

根拠: [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §4·§7.1·§18.6 · [x00cc](x00cc-vcs-push.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| `POST …/negotiate` / `…/push` は owner JWT；意味は Elixir `Vcs.negotiate/3` / `push/3` と同じ | SSH transport；パスワード Git |
| push HTTP の object bytes は base64；不足 blob は既存 `POST /blobs`；body 超過は 413 | clone / fetch / pull |
| `alchemy-vcs-store`: init / put_object / commit / push；`old_commit_hash` = remote-tracking tip（`refs/remotes/origin/<branch>`）；成功後に tip 更新 | Save UI；shared-sim 部分木変換 |
| push は current branch の tip のみ；object 収集は remote tip までの差分（≤512）；欠落 object は fail-closed | 1 hop 限定（複数ローカルコミットを一括 push 可） |
| ローカル object は content-addressed；hash mismatch はエラー | branch UI；fork / PR；`:purchased` |

---

## 設計メモ

- Router: `:user_auth` 配下 `POST /api/v1/vcs/repos/:repo_id/negotiate|push`
- Controller → `Assets.Vcs`；検証ロジックは x00cc のまま
- クレート: `client/vcs-store`（MPL-2.0）；codec は `alchemy-protocol::vcs_object`
- tip シードは `put_object` + `set_head` + `set_remote_tip`（clone 代替の最小）。Save UI 以前は呼び出し側（テスト / 将来の app 配線）の責務。漏れ時の `cas_mismatch` は仕様どおり fail-closed
- マルチコミット push の CAS は remote-tracking（HEAD の parent ではない）
- push 成功後の tracking IO 失敗は `TrackingUpdateFailed { tip }`（サーバは進済み；修復は `set_remote_tip(…, Some(tip))`）
- 応答 tip ≠ リクエスト tip は `RemoteTipMismatch { expected, actual }`（tracking 未更新；修復は `set_remote_tip(…, Some(expected))`。正直サーバは request tip へ進む；response tip は信用しない）

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice 文書 | done |
| 1 | HTTP negotiate/push + ConnCase | done |
| 2 | client vcs-store init/commit/push | done |
| 3 | テスト + docs 鮮度 | done |

---

## 完了条件

1. 認証付き negotiate/push が成功し、非オーナー / protected / non-FF で拒否
2. vcs-store がローカル commit し、モック Remote で push 手順を踏む
3. Slice / README / draft §18.6 / assets-server.md 鮮度一致

## やらない

- clone / fetch / pull / branch UI
- Save UI / shared-sim 変換
- fork / PR / SSH
- Rust↔実サーバ e2e（Elixir HTTP テストで足りる）

← [目次](../README.md) · [x00cc](x00cc-vcs-push.md) · [assets-vcs-draft](../../0_doc/architecture/assets-vcs-draft.md)
