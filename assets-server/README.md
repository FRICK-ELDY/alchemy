# assets-server（Slice C-A）

ローカル CAS（BLAKE3）+ SQLite メタ + 固定 Quota（1GiB）。auth Bearer 必須。  
BLAKE3 は `b3sum` CLI 経由（Docker イメージに同梱。ホスト CI は `cargo install b3sum`）。

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d
# http://localhost:4004/health
```

## 例

```bash
bash ../.scripts/smoke-assets.sh
```

設計: [`.workspace/0_doc/architecture/assets-server.md`](../.workspace/0_doc/architecture/assets-server.md)
