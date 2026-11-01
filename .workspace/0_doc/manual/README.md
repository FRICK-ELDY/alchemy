# Manual（運用手順）

アーキテクチャ正本は [`../architecture/`](../architecture/)。ここは **手元での立ち上げ・公開** の手順。

| 文書 | 内容 |
|:---|:---|
| [cloudflare-tunnel-windows.md](cloudflare-tunnel-windows.md) | Cloudflare Tunnel（Windows）— auth / world / assets の HTTPS 公開（x001a） |
| [relay-server-wan.md](relay-server-wan.md) | WAN 正本 — zenohd（TLS+mTLS）+ world 短命 client 証明書（x0066）。Relay Agent は互換 |
| [relay-server-dns.md](relay-server-dns.md) | 追加章 — DNS Only（グレー雲）+ Let's Encrypt サーバ証明書自動更新 |
| [assets-cas-r2.md](assets-cas-r2.md) | assets CAS → Cloudflare R2（x001c）+ GET 署名 URL 302（x001d） |
