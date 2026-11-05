# Cursor プロジェクトスキル

| スキル | 用途 |
|:---|:---|
| [`rust-best-practices/`](rust-best-practices/) | Rust 実装・レビュー時のイディオム |
| [`security-audit/`](security-audit/) | コードベースの信頼境界ベース監査（Cloudflare upstream） |

`security-audit` の vendored 記録（commit / 日付）は [`security-audit/README.md`](security-audit/README.md) を正とする。

## セキュリティレビューの役割分担

| 依頼の種類 | 使うもの | 備考 |
|:---|:---|:---|
| PR / ブランチ差分のセキュリティレビュー | Cursor `/review-security` | 変更 diff 向け。高速 |
| 実装後の一般レビュー | [`.cursor/rules/code-review.mdc`](../rules/code-review.mdc) | 正確性・性能・設計中心 |
| セキュリティ質問・局所調査 | `security-audit` **Guidance** | 成果物ディレクトリは作らない |
| 明示的な audit / pen-test / 包括レビュー | `security-audit` **Full audit** | 6 フェーズ。出力はリポジトリ外 |

例:

```
security audit client/trust — profile quick, scoped
```

```
find security vulnerabilities in auth-server/
```

Full audit の既定出力先は `~/security-audit-skill/<repo>/run-N/`。リポジトリ内に書く場合は gitignore 済みパスを明示すること。監査成果物はコミットしない。

前提: Full audit の `findings.json` / `coverage-ledger.json` 検証には **Node.js が PATH 上で必要**（`node validate-*.cjs ...`）。Windows でもスキーマ検証 CLI は完走させる想定（Alchemy フォールバックあり。詳細は [`security-audit/README.md`](security-audit/README.md)）。`npx skills` が使えない環境では上流を本ディレクトリへ手動同梱する（現状の配置）。
