# security-audit（Cursor skill）

[cloudflare/security-audit-skill](https://github.com/cloudflare/security-audit-skill) の `skills/security-audit` を Cursor プロジェクトスキルとして同梱したものです（MIT。`LICENSE` / `UPSTREAM-README.md` 参照）。

| 項目 | 値 |
|:---|:---|
| Upstream | https://github.com/cloudflare/security-audit-skill |
| Commit | `c1c8a8c1471069fb0e188eeaff69b8e8db6564a8`（`main`） |
| Vendored | 2026-11-05 |

| パス | 内容 |
|:---|:---|
| `SKILL.md` | 本体（モード・原則・フル監査セットアップ） |
| `*.md` companions | 攻撃クラス・フェーズ手順 |
| `report-schema.json` / `validate-*.cjs` | findings / coverage-ledger 検証（Node.js） |
| `host-read-within-limit.cjs` | Alchemy 共有入力読取（no-follow または lstat フォールバック） |
| `cli-test-helpers.cjs` | バリデータテスト共有（WARNING strip 等） |

## Alchemy での使い分け

詳細は [../README.md](../README.md)。要約:

- **PR 差分** → Cursor `/review-security`（このスキルで置換しない）
- **質問・局所** → Guidance mode
- **明示 audit** → Full audit（出力はリポジトリ外。初回は `quick` / scoped 推奨）

Alchemy 向け companion の優先例: `WEB-PROTOCOL-AND-AUTH.md`、`PROTOCOLS-RPC-AND-MESSAGING.md`、`MEMORY-SAFETY-AND-BINARY.md`、`SUPPLY-CHAIN-AND-RELEASE.md`、`DATA-ISOLATION-AND-LIFECYCLE.md`。

競合時は [`.cursor/rules/guardrails.mdc`](../../rules/guardrails.mdc) と [`.cursor/rules/license-and-boundaries.mdc`](../../rules/license-and-boundaries.mdc) が優先します。

上流のまま `disable-model-invocation` は付けない。description は広いが、Full audit は明示依頼時のみ（Guidance 既定）。巨大 companion は Full audit / 明示スコープ時に必要なものだけ読む。

## Node / Windows（Full audit 完走想定）

Windows 開発機でも Full audit の **スキーマ検証は必須**（検証スキップ禁止）。

- Node.js を PATH に入れる（winget `OpenJS.NodeJS.LTS` → 通常 `C:\Program Files\nodejs\`）。新しいシェルで `node -v` が通ることを確認。
- 上流は `O_NOFOLLOW` 欠如時に CLI 入力を拒否する。**Alchemy 差分**: 共有 [`host-read-within-limit.cjs`](host-read-within-limit.cjs) で `lstat`+`readFileSync` フォールバックを提供し、両バリデータが同じ CLI 手順で PASS できるようにしている（常に stderr へ WARNING。TOCTOU 耐性は POSIX no-follow より弱い）。
- `validate-*.test.cjs`: 文書受け入れ／上限／UTF-8 等の CLI 回帰はフォールバック経路でも実行。symlink／FIFO／制約ヒープなど no-follow・nonblock 固有だけ `HAS_SAFE_INPUT_OPEN` で skip。
- `report-schema.json` はスキル隣接の信頼済みファイルとして `readFileSync`、監査入力 JSON だけ厳格 open／フォールバック — 入口が二系統なのは意図的。

煙テスト成果物例: `%USERPROFILE%\security-audit-skill\alchemy\run-1\`（リポジトリ外）。

## 更新

上流を確認し、`skills/security-audit/` を差し替えたうえで:

1. 本 README の Commit / Vendored を更新する
2. Alchemy 差分（`host-read-within-limit.cjs` / `cli-test-helpers.cjs`、両バリデータ、CLI 文書系＋fallback symlink テスト、各 `.md` 注記）を再適用する。**`host-read-within-limit.cjs` をコミット漏れしないこと**（`validate-*.cjs` が require）
3. 使い分け注記を残す
