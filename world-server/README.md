# world-server（Slice B）

短命 Room Token の発行と JWKS 配信。DB なし（部屋はプロセス内レジストリ）。

## 起動

```bash
docker compose up -d
# http://localhost:4003/health
# http://localhost:4003/.well-known/jwks.json
```

## 例

```bash
curl -s -X POST http://127.0.0.1:4003/api/v1/rooms \
  -H 'content-type: application/json' \
  -d '{"room_id":"local"}'

curl -s -X POST http://127.0.0.1:4003/api/v1/rooms/local/tokens \
  -H 'content-type: application/json' \
  -d '{"role":"host","sub":"dev"}'
```

設計の正本: [`.workspace/0_doc/architecture/world-server.md`](../.workspace/0_doc/architecture/world-server.md)
