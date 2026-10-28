# 技術選定カタログ

Alchemy が採用している **言語・基盤・ライブラリ** と、その **宣言バージョン／採用理由** をまとめる。

| 文書 | 内容 |
|:---|:---|
| [stack.md](stack.md) | Elixir / Rust / Zenoh / Protobuf など、層をまたぐ技術選定 |
| [rust.md](rust.md) | Rust クレート（`client/`・`protocol/`・`launcher/`） |
| [elixir.md](elixir.md) | Elixir / Hex 依存（`auth-server`・`world-server`・`assets-server`・共有） |

> フォルダ名は **`tech-choices`**（`deps` ではない）。Mix の `**/deps/` gitignore で文書が隠れないようにする。

## バージョンの読み方

- **宣言**: `Cargo.toml` / `mix.exs` / Dockerfile に書いた範囲（`1.10`・`~> 1.8` など）
- **固定**: `Cargo.lock` / `mix.lock` が解決した具体版。CI・再現ビルドの正本
- このフォルダの表は主に **宣言** を載せ、主要なものだけ固定版を併記する

## 更新方針

- 依存を追加・削除・メジャー上げしたら、該当表と理由を同じ PR で直す
- 「なぜ入れたか」が書けない依存は、入れる前に [guardrails](../../../.cursor/rules/guardrails.mdc) どおり **人に回す**
- アーキテクチャ上の保証／非保証の正本は [architecture/](../architecture/) 側。ここは **選定の説明** に留める
