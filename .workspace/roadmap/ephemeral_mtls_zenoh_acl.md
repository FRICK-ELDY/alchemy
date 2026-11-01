# Ephemeral mTLS + Zenoh ACL（設計ドラフト）

**正本は Slice [`slices/x0066-ephemeral-mtls-world-issue.md`](slices/x0066-ephemeral-mtls-world-issue.md) に移行した。**

確定方針（実装）:

- WAN: world-server が openssl で短命 client 証明書を発行（CSR Subject 無視）
- ローカル tcp / Relay Agent（x004e）は併存
- キー式は現行 `alchemy/dev/room/{room_id}/{wire_cap}/…`（第2壁）。ACL CN が第1壁
- Elixir に `x509` Hex は足さない。Rust は `rcgen` のみ追加

詳細・完了条件・Phase は x0066 を見ること。
