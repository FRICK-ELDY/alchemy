# x0078 — mTLS 鍵 temp の権限・掃除（Windows）

> **ID:** `x0078` · **slug:** `mtls-temp-acl-sweep` · **status:** `done` · **area:** `[client, zenoh, docs]` · **depends_on:** `[x0066, x0068]`

目標: improvement-plan **P2-9** — `%TEMP%/alchemy-mtls`（`RELAY_MTLS_WORK_DIR`）に書く短命 PEM を、**Unix 0600 相当の権限**にし、**期限切れ／置換済みの孤児ファイルを掃除**する。

根拠: [backlog-priority-2026-11-04.md](../backlog-priority-2026-11-04.md) **P3** · [improvement-plan-2026-10-30.md](../../0_doc/evaluation/improvement-plan-2026-10-30.md) **P2-9** · [opus-specific-weaknesses-2026-10-30.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-10-30.md)

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 新規書き込み PEM は Unix `0600` / Windows 現行ユーザーのみ DACL（継承切断） | DPAPI / TPM 保管；メモリのみ（Zenoh 1.x がパス要求） |
| 権限設定失敗は fail-closed（ファイルを残して「緩い ACL」で続行しない） | `icacls` / `USERNAME` 欠落環境での接続継続 |
| 置換時に直前の **managed** PEM を削除（静的 `ZENOH_TLS_CONNECT_*` は触らない） | プロセス外の静的証明書の寿命管理 |
| install 時に work dir 内の古い managed `*.pem`（mtime > soft TTL + skew）を掃除 | 他プロセスが同時に触る同一 work dir の完全排他 |
| 新規 Hex / Cargo 依存なし（Windows は `icacls`） | `windows-sys` ネイティブ ACL API |

## 設計

1. **`write_secret`**: Unix `chmod 0600`；Windows `icacls /inheritance:r /grant:r USER:(R,W)`。失敗時は当該ファイルを消して `Err`
2. **`install_identity_env`**: 既存どおり、置換された managed パスのみ `remove_file`
3. **`sweep_stale_mtls_temps`**: `write_identity_pems` 後に work dir を走査。keep（今回の cert/key）以外で mtime が `WORLD_MTLS_SOFT_TTL_SECS + 120` を超えた `*.pem` を削除（best-effort）

## 本 Slice に含めない

| 項目 | 予定 |
|:---|:---|
| DemoScene 共有テストベクタ | 別 Slice |
| auth/world/relay metrics + `jti` 相関 | 別 Slice |
| scene despawn | 別 Slice |
| world-server `MtlsIssuer` の openssl scratch（既に `after` 掃除済み） | 変更しない |

## 完了条件

1. Windows で新規 key/cert PEM にユーザー限定 DACL；Unix は 0600 維持
2. 再 install で直前 managed PEM が消え、古い孤児 `*.pem` も sweep される
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
