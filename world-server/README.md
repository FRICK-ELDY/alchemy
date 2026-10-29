# world-server（Slice B.5）

短命 Room Token の発行と JWKS 配信。DB なし（部屋はプロセス内レジストリ）。  
Room API は **auth-server の Bearer JWT 必須**（匿名は既定 OFF）。

## 起動

auth-server を先に上げてから:

```bash
docker compose up -d
# http://localhost:4003/health
# AUTH_JWKS_URL → host.docker.internal:4002
```

## 例

```bash
ACCESS=$(curl -s -X POST http://127.0.0.1:4002/api/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"identifier":"alice","password":"Password1"}' \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["access_token"])')

curl -s -X POST http://127.0.0.1:4003/api/v1/rooms \
  -H 'content-type: application/json' \
  -H "authorization: Bearer $ACCESS" \
  -d '{"room_id":"local"}'

# x003e: session_pub (Ed25519 32B, base64url no pad) required — embeds as JWT cnf.spk
SPK=$(openssl rand 32 | openssl base64 -A | tr '+/' '-_' | tr -d '=')
curl -s -X POST http://127.0.0.1:4003/api/v1/rooms/local/tokens \
  -H 'content-type: application/json' \
  -H "authorization: Bearer $ACCESS" \
  -d "{\"role\":\"host\",\"session_pub\":\"${SPK}\"}"
```

設計の正本: [`.workspace/0_doc/architecture/world-server.md`](../.workspace/0_doc/architecture/world-server.md)
