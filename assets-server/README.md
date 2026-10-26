# assets-server

ローカル CAS（BLAKE3）または **S3 互換（Cloudflare R2）** + SQLite メタ + 固定 Quota（1GiB）。auth Bearer 必須。  
BLAKE3 は `b3sum` CLI 経由（Docker イメージに同梱。ホスト CI は `cargo install b3sum`）。

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d
# http://localhost:4004/health
```

既定は `ASSETS_CAS_BACKEND=local`。R2: [manual/assets-cas-r2.md](../.workspace/0_doc/manual/assets-cas-r2.md) · [`.env.example`](.env.example)

## 例

```bash
bash ../.scripts/smoke-assets.sh
# R2（サーバー側を s3 にしたうえで）:
# bash ../.scripts/smoke-assets-r2.sh
```

設計: [`.workspace/0_doc/architecture/assets-server.md`](../.workspace/0_doc/architecture/assets-server.md) · Slice [x001c](../.workspace/roadmap/slices/x001c-assets-cas-r2.md)
