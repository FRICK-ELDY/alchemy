# x0078 — mTLS 鍵 temp の権限・掃除（Windows）

> **ID:** `x0078` · **slug:** `mtls-temp-acl-sweep` · **status:** `done` · **area:** `[client, zenoh, docs]` · **depends_on:** `[x0066, x0068]`

目標: improvement-plan **P2-9** — `%TEMP%/alchemy-mtls`（`RELAY_MTLS_WORK_DIR`）に書く短命 PEM を、**Unix 0600 相当の権限**にし、**期限切れ／置換済みの孤児ファイルを掃除**する。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P3** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P2-9** · [opus-specific-weaknesses-2026-10-30.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-10-30.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 新規書き込み PEM は Unix `0600` / Windows 現行ユーザーのみ DACL（継承切断）。**PEM 本体は restrict 後に書く** | DPAPI / TPM 保管；メモリのみ（Zenoh 1.x がパス要求）；create-with-ACL の OS 原子性（空ファイルが一瞬存在する） |
| 権限設定失敗は fail-closed（ファイルを残して「緩い ACL」で続行しない）。非 unix/windows も fail-closed | `icacls` / `USERNAME` 欠落環境での接続継続 |
| 置換時に直前の **managed** PEM を削除（静的 `ZENOH_TLS_CONNECT_*` は触らない） | プロセス外の静的証明書の寿命管理 |
| install 時に古い managed `{client\|world-mtls}-*.{cert\|key}.pem`（mtime > soft TTL + skew）のみ掃除。非 managed 名は触らない | 他プロセスが同時に触る同一 work dir の完全排他 |
| cert 成功・key 失敗時は cert もロールバック | — |
| 新規 Hex / Cargo 依存なし（Windows は `icacls`） | `windows-sys` ネイティブ ACL API |

## 設計

1. **`write_secret`**: 空ファイル作成 → Unix `chmod 0600` / Windows `icacls /inheritance:r /grant:r USER:(R,W)` → PEM 書き込み。失敗時は当該ファイルを消して `Err`
2. **`install_identity_env`**: 既存どおり、置換された managed パスのみ `remove_file`
3. **`sweep_stale_mtls_temps`**: `write_identity_pems` 後に work dir を走査。`client-` / `world-mtls-` かつ `*.cert.pem` / `*.key.pem` で、keep 以外かつ mtime が `WORLD_MTLS_SOFT_TTL_SECS + 120` 超を削除（best-effort）

## 本 Slice に含めない

| 項目 | 予定 |
|:---|:---|
| DemoScene 共有テストベクタ | 別 Slice |
| auth/world/relay metrics + `jti` 相関 | 別 Slice |
| scene despawn | 別 Slice |
| world-server `MtlsIssuer` の openssl scratch（既に `after` 掃除済み） | 変更しない |

## 完了条件

1. Windows で新規 key/cert PEM にユーザー限定 DACL；Unix は 0600 維持
2. 再 install で直前 managed PEM が消え、古い managed `{client|world-mtls}-*.{cert|key}.pem` も sweep される（非 managed は残る）
3. 静的 env パスは削除されない（既存テスト維持）
4. `cargo test -p zenoh-connect` 緑
5. Slice / backlog / README 鮮度一致

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | done |
| 1 | Windows ACL + fail-closed | done |
| 2 | stale PEM sweep + テスト | done |
| 3 | docs 鮮度 | done |

← [目次](../README.md)
