# security-audit（Cursor skill）

[cloudflare/security-audit-skill](https://github.com/cloudflare/security-audit-skill) の `skills/security-audit` を Cursor プロジェクトスキルとして同梱したものです（MIT。`LICENSE` / `UPSTREAM-README.md` 参照）。

| パス | 内容 |
|:---|:---|
| `SKILL.md` | 本体（モード・原則・フル監査セットアップ） |
| `*.md` companions | 攻撃クラス・フェーズ手順 |
| `report-schema.json` / `validate-*.cjs` | findings / coverage-ledger 検証（Node.js） |

## Alchemy での使い分け

詳細は [../README.md](../README.md)。要約:

- **PR 差分** → Cursor `/review-security`（このスキルで置換しない）
- **質問・局所** → Guidance mode
- **明示 audit** → Full audit（出力はリポジトリ外。初回は `quick` / scoped 推奨）

Alchemy 向け companion の優先例: `WEB-PROTOCOL-AND-AUTH.md`、`PROTOCOLS-RPC-AND-MESSAGING.md`、`MEMORY-SAFETY-AND-BINARY.md`、`SUPPLY-CHAIN-AND-RELEASE.md`、`DATA-ISOLATION-AND-LIFECYCLE.md`。

競合時は [`.cursor/rules/guardrails.mdc`](../../rules/guardrails.mdc) と [`.cursor/rules/license-and-boundaries.mdc`](../../rules/license-and-boundaries.mdc) が優先します。

## Node / Windows メモ

- Full audit のスキーマ検証には Node.js が必要（`validate-*.cjs`）。
- Windows では `O_NOFOLLOW` / `O_NONBLOCK` が無いことが多く、**CLI 起動**（`node validate-findings.cjs path`）はファイル読み取りを拒否しうる。その場合は同梱テスト（`node --test validate-*.test.cjs`）か、`validateDocument()` 経由で検証する。
- 導入時の scoped 煙テスト成果物例: `%USERPROFILE%\security-audit-skill\alchemy\run-1\`（リポジトリ外）。

## 更新

上流を確認し、`skills/security-audit/` 配下を差し替える。Alchemy 用の本 README の使い分け注記は残す。
