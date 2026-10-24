# x0003 — assets-server 最小（完了）

> **ID:** `x0003` · **旧称:** Slice C-A · **slug:** `assets-cas-quota` · **status:** done · **area:** `[assets, auth]` · **depends_on:** `[x0002]`

目標: auth JWT 必須でローカル CAS に Blob を put/get し、固定 Quota（1GiB）を守る。  
正本: [assets-server.md](../../0_doc/architecture/assets-server.md)

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | `assets-server` scaffold（Phoenix / Ecto SQLite / Docker） | done |
| 1 | BLAKE3 CAS + BlobRef / QuotaLedger + auth Bearer | done |
| 2 | smoke + CI | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| auth Bearer 必須の upload / download | client の GLB wgpu 描画（→ C-B） |
| BLAKE3 キーのローカル CAS | R2 / NAS 切替 |
| SQLite メタ（BlobRef / QuotaLedger） | Vault / Folder / Commit VCS |
| 固定 Quota 1GiB（超過は 413） | Ash 本格ドメイン / pgvector / Origin Cert |

## 起動

```bash
cd auth-server && docker compose up -d && cd ..
cd assets-server && docker compose up -d && cd ..
bash .scripts/smoke-assets.sh
```

← [目次](../README.md)
