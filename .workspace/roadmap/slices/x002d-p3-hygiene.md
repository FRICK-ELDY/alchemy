# x002d — P3 衛生（A+C）

> **ID:** `x002d` · **slug:** `p3-hygiene` · **status:** `done` · **area:** `[auth, assets, protocol, ci, docs]` · **depends_on:** `[x002c]`

目標: 評価書 P3 の **門番衛生（A）** と **CI/依存衛生（C）** を縦に潰す。ワイヤ・PROTOCOL_PIN は据え置き。

根拠: [improvement-plan-2026-10-26.md](../../0_doc/evaluation/improvement-plan-2026-10-26.md) P3-1/2/3/6/12/13

## 含む / 含まない

| 含む | 含まない（後続） |
|:---|:---|
| P3-1 Quota 原子的条件更新 | P3-4 `alchemy_auth_client` |
| P3-2 Refresh `FOR UPDATE` / 条件付き revoke | P3-5 ClientIp / CF-Connecting-IP |
| P3-3 `/blobs` で Plug.Parsers を外す | P3-8 Mutex poison fatal |
| P3-6 `mix hex.audit` / `cargo deny` CI gate | P3-9 UI HTTP ワーカー |
| P3-12 誤コミット `-o` 削除 | P3-10 room_id 正本統一 |
| P3-13 protoc 全 proto + Rust cache | P3-14 undocumented_unsafe deny |

完了済み（触れない）: P3-7（x001f）、P3-11（x0029）。

## 設計方針

| ID | 層 | 内容 |
|:---|:---|:---|
| P3-1 | assets | `UPDATE … WHERE used+? <= limit`；超過は rollback |
| P3-2 | auth | refresh を TX + `FOR UPDATE`；revoke は `revoked_at IS NULL` 条件 |
| P3-3 | assets | Endpoint から `Plug.Parsers` 削除（blob は raw `read_body`） |
| P3-6 | ci | `mix hex.audit`（auth/world/assets）+ `cargo deny`（client）；hackney HIGH 解消 |
| P3-12 | repo | ルート `-o` を `git rm` |
| P3-13 | ci | protoc 4 proto；Rust job に `Swatinem/rust-cache` |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 並行 upload でも `used_bytes` が limit を超えない | 完全直列化・公平スケジューリング |
| refresh 競合で二重発行しない；再利用は既存 grace のまま | DPoP / refresh のワイヤ変更 |
| `Content-Type: application/json` の POST `/blobs` でも body が読める | JSON API の追加 |
| CI で `mix hex.audit` と `cargo deny` がゲートになる | SBOM / provenance 署名 |
| 4 proto が descriptor に載る；Rust CI に cache | ローカル Windows 必須 protoc |

## PIN

据え置き（`alchemy-protocol/0.14.0-x002c`）。ワイヤスキーマ不変。

## 成功条件

1. assets: 条件付き quota UPDATE + 回帰テスト緑  
2. auth: refresh が TX + FOR UPDATE + 条件 revoke、競合テスト緑  
3. assets: JSON Content-Type でも blob upload 成功  
4. CI / `ci.sh`: `mix hex.audit`（3 app）と `cargo deny` が走る  
5. ルートに `-o` が無い  
6. protoc が 4 proto をコンパイル；Rust CI に cache ステップ  

## 実装メモ

- assets: ExAws HTTP → `Assets.CAS.ExAwsReq`（hackney 削除）；bandit 1.14 / jose 1.11.13
- auth: ash ~> 3.37 + `default_string_length_count: :codepoints`（hex.audit 緑のため）
- cargo-deny: `client/deny.toml`；rsa / lz4_flex は文書化 ignore；unmaintained はゲート外

← [目次](../README.md)
