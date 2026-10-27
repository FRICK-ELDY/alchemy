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
| 登録・ログイン・Refresh・ログアウト | グループ / Block / フォロー承認制 |
| Follow（有向）+ Friendship（相互フォロー時のみ申請→accept）（x0033） | 関係グラフを JWT に載せること |
| unfollow は同一トランザクションで Follow 削除＋Friendship 解除 | Room Token の即時失効（world TTL） |
| S2S `GET /internal/v1/access`（visibility 判定・x0034/x0035） | Room の `visibility` 永続（world） |
| prod は `AUTH_INTERNAL_TOKEN` 必須（未設定は起動失敗） | 既定 `"dev-internal-token"` を prod に残すこと |


## 契約

- JWT: RS256、`iss` / `aud` / `kid`（JWK thumbprint）
- engine 側検証契約は alchemy-engine の `jwt-jwks-engine-contract.md` を参照（本リポへの全文移設は後続）

### Social（x0033）

| Method | Path | Auth | 内容 |
|:---|:---|:---|:---|
| POST | `/api/v1/social/follows/:user_id` | Bearer | follow |
| DELETE | `/api/v1/social/follows/:user_id` | Bearer | unfollow（pending / accepted とも解除） |

| POST | `/api/v1/social/friend-requests/:user_id` | Bearer | Mutual 必須。非 Mutual → 422 `not_mutual` |
| POST | `/api/v1/social/friend-requests/:user_id/accept` | Bearer | addressee のみ |
| POST | `/api/v1/social/friend-requests/:user_id/reject` | Bearer | 当事者 |
| DELETE | `/api/v1/social/friends/:user_id` | Bearer | unfriend |

### S2S Access（x0034）

| Method | Path | Auth | 内容 |
|:---|:---|:---|:---|
| GET | `/internal/v1/access?owner_sub=&viewer_sub=&visibility=` | `x-alchemy-internal-token` | `{allowed: bool}` |

`visibility`: `lan` \| `private` \| `friends` \| `friends_plus` \| `followers` \| `followers_plus` \| `public`  
（`lan` の経路判定は world。auth は関係ゲートを常に open。）


## 起動（Docker）

ローカル開発:

```powershell
cd auth-server
docker compose up -d
# http://localhost:4002/health
# http://localhost:4002/.well-known/jwks.json
```

**公開 / Tunnel（x001e）:** `docker-compose.prod.yml`（release）のみ。`MIX_ENV=dev` を Cloudflare の後ろに置かない（`/dev/mailbox` 乗っ取り経路）。手順: [`edge/cloudflared/README.md`](../../../edge/cloudflared/README.md)。

シードアカウント（`priv/repo/seeds.exs`・**dev のみ**）: `alice` / `bob` / `carol` / `admin`、パスワード `Password1`。prod compose は seed しない。

**推奨起動は全 OS とも Docker Compose**。  
Windows ホスト直の `mix compile` は `argon2_elixir` に Visual C++ Build Tools（nmake）が必要。

ワイヤ契約の正本は [`protocol/`](../../../protocol/)（client 外）。
