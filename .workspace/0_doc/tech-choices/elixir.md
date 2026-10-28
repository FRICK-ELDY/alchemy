# Elixir / Hex 依存の選定

出典: 各サービスの `mix.exs`。ランタイムは Docker `elixir:1.19-otp-28-alpine`（Elixir `~> 1.19` / OTP 28）。

## サービス別の置き場

| アプリ | パス | 役割（要約） |
|:---|:---|:---|
| `auth` | `auth-server/` | アカウント・JWT/JWKS・ソーシャル台帳の SSoT |
| `world` | `world-server/` | 案内所・Room Token・ManifestSeal・公開範囲 |
| `assets` | `assets-server/` | CAS メタ・Quota・local/R2 門番 |
| `alchemy_auth_client` | `shared/alchemy_auth_client/` | JWKS キャッシュとユーザー JWT 検証の共有 |

## 共通（ほぼ全サービス）

| パッケージ | 宣言 | 利用者 | 理由 |
|:---|:---|:---|:---|
| `phoenix` | `~> 1.8.8` | auth, world, assets | HTTP API・Plug パイプライン |
| `bandit` | `~> 1.14` | 同上 | Phoenix 1.8 既定寄りの HTTP サーバ |
| `jason` | `~> 1.2` | 同上 | JSON 符号化（制御面 API） |
| `joken` | `~> 2.6` | 全サービス + 共有 | JWT の発行／検証 |
| `jose` | `~> 1.11`（auth/world は `1.11.13`） | 同上 | JWK / 暗号プリミティブ（JWKS） |
| `req` | `~> 0.5` | 同上 | 外向き HTTP（JWKS 取得・S2S）。httpoison 等は使わない |
| `telemetry_metrics` / `telemetry_poller` | `~> 1.0` | auth, world, assets | メトリクス基盤 |
| `credo` | `~> 1.7`（dev/test） | 各サービス | 静的解析（`mix precommit`） |

## auth-server 固有

| パッケージ | 宣言 | 理由 |
|:---|:---|:---|
| `ash` | `~> 3.37` | ユーザー／トークン等のドメインをリソースとして定義 |
| `ash_postgres` | `~> 2.6` | Ash ↔ PostgreSQL |
| `phoenix_ecto` / `ecto_sql` | `~> 4.5` / `~> 3.13` | Ecto 統合と SQL |
| `postgrex` | `>= 0.0.0` | PostgreSQL ドライバ（アイデンティティの本番寄り永続） |
| `argon2_elixir` | `~> 4.0` | パスワードハッシュ（メモリハード） |
| `remote_ip` | `~> 1.2` | 信頼プロキシ配下のクライアント IP（レート制限） |
| `swoosh` | `~> 1.16` | メール（パスワード再設定等） |
| `phoenix_live_dashboard` | `~> 0.8.3` | 運用ダッシュボード |
| `gettext` / `dns_cluster` | `~> 1.0` / `~> 0.2.0` | i18n・クラスタ名解決（Phoenix 生成物の延長） |

## world-server 固有

| パッケージ | 宣言 | 理由 |
|:---|:---|:---|
| `ash` | `~> 3.0` | Room / 公開範囲など門番ドメイン |
| `ash_sqlite` | `~> 0.2` | Ash ↔ SQLite（Stage-1） |
| `ecto_sqlite3` | `~> 0.17` | SQLite アダプタ |
| `phoenix_ecto` / `ecto_sql` | `~> 4.5` / `~> 3.13` | 永続化 |
| `alchemy_auth_client` | path | auth JWT を世界側で検証（JWKS 共有実装） |
| `remote_ip` | `~> 1.2` | クライアント IP |

空間同期や物理は持たない（Pure Elixir の案内所・門番）。

## assets-server 固有

| パッケージ | 宣言 | 理由 |
|:---|:---|:---|
| `ecto_sql` / `ecto_sqlite3` | `~> 3.12` / `~> 0.17` | Blob メタ・Quota の台帳（Ash は未採用） |
| `alchemy_auth_client` | path | Bearer JWT 検証 |
| `ex_aws` / `ex_aws_s3` | `~> 2.5` | R2（S3 互換）バックエンド |
| `sweet_xml` | `~> 0.7` | AWS XML 応答の解析（ex_aws 系） |

## shared: alchemy_auth_client

| パッケージ | 宣言 | 理由 |
|:---|:---|:---|
| `joken` / `jose` / `req` | 上記と同様 | JWKS 取得キャッシュとユーザー JWT 検証を world / assets で共有 |
| `bypass` | `~> 2.1`（test） | HTTP 境界のユニットテスト |

## 意図的な差

| 項目 | auth | world | assets |
|:---|:---|:---|:---|
| DB | PostgreSQL | SQLite | SQLite |
| Ash | あり | あり | なし（薄い Ecto） |
| オブジェクト本体 | 持たない | 持たない | CAS（local または R2）。DB に GLB を置かない |

PostgreSQL は「ユーザーが絶対 SSoT」の auth に寄せ、world / assets は個人運用の Stage-1 を SQLite で軽く保つ。
