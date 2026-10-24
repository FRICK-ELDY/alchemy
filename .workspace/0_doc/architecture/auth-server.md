# auth-server

> **正本コード**: リポジトリ直下 [`auth-server/`](../../../auth-server/)  
> **ステータス**: alchemy-engine から移植済み（Slice A）

## 図上の位置

[overview.md](overview.md) の運営側コア — JWT / JWKS の SSoT。

## 保証すること / しないこと

| 保証する | 保証しない |
|:---|:---|
| ユーザー ID（JWT `sub` = UUID）の発行・検証 | Room Token（world-server の責務） |
| `GET /.well-known/jwks.json` による公開鍵配信 | Zenoh ACL / リレー割当 |
| 登録・ログイン・Refresh・ログアウト | Friendship / グループ（将来） |

## 契約

- JWT: RS256、`iss` / `aud` / `kid`（JWK thumbprint）
- engine 側検証契約は alchemy-engine の `jwt-jwks-engine-contract.md` を参照（本リポへの全文移設は後続）

## 起動（Docker）

```powershell
cd auth-server
docker compose up -d
# http://localhost:4002/health
# http://localhost:4002/.well-known/jwks.json
```

シードアカウント（`priv/repo/seeds.exs`）: `alice` / `bob` / `carol` / `admin`、パスワード `Password1`。

**推奨起動は全 OS とも Docker Compose**。  
Windows ホスト直の `mix compile` は `argon2_elixir` に Visual C++ Build Tools（nmake）が必要。

ワイヤ契約の正本は [`protocol/`](../../../protocol/)（client 外）。
