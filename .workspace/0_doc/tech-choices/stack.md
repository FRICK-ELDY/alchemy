# スタック層の技術選定

層をまたぐ技術の採用理由。保証／非保証の正本は [architecture/overview.md](../architecture/overview.md)。

## Elixir（+ OTP）

| | |
|:---|:---|
| **場所** | `auth-server/`・`world-server/`・`assets-server/`（Core Profile: アイデンティティ・門番・永続台帳） |
| **宣言** | Elixir `~> 1.19` · Docker イメージ `elixir:1.19-otp-28-alpine` |
| **理由** | 長寿命の HTTP/API を、監督木とプロセス分離で回すのに向く。auth / world / assets は **定義と門番**（JWT/JWKS・Room Token・ManifestSeal・CAS メタ）であり、60 Hz の空間シミュレーションではない。 |
| **明示的に選ばないもの** | 空間シミュ用 NIF、独自 UDP、空間同期用 Phoenix Channel。これらは廃止し、Zenoh + Pure Rust host に一本化した。 |

OTP の障害隔離と、Phoenix / Ash による「永続権威＋監査」の書きやすさが、client / host のホットパスに埋め込むより合う。

## Rust

| | |
|:---|:---|
| **場所** | `client/app`・`client/host-server`・`client/shared-sim`・`protocol/`・`launcher/`・probe 類 |
| **宣言** | Edition 2021 · ワークスペースは `client/`（加えて独立の `protocol/`・`launcher/`） |
| **理由** | 空間の **実行**: Rapier tick、SoA、wgpu / OpenXR 描画、ワイヤ encode/decode、Ed25519。データ指向・ゼロコピー方針と、Desktop → VR →（将来）Wasm への単一コードベースに合う。 |
| **分割** | **host-server** = 空間権威プロセス、**client** = エンドポイントのみ（ルーターではない）、**protocol** = 独立したワイヤ SSoT クレート。 |

## Zenoh

| | |
|:---|:---|
| **場所** | 実行時: host / client の `zenoh` クレート · 運用: `zenohd`（`zenoh-router/`） |
| **宣言** | Rust クレート `zenoh = "1.10"`（固定 ≈ `1.10.1`） |
| **理由** | Manifest / Presence / RenderFrame / Input / liveliness / snapshot を **一本のトランスポート** に載せる。Pub/Sub と queryable が部屋スコープのキーに合う。LAN と WAN（ConoHa ブリッジ／将来の Open Relay）を同じモデルで扱える。 |
| **明示的に選ばないもの** | 空間同期用の独自 UDP と Phoenix Channel。OSC はクライアント局所の **デバイス入力エッジ** に留め、検証後に Protobuf 化して Zenoh へ送る。 |

防御（mTLS・ACL）は発展途上。[trust-boundary.md](../architecture/trust-boundary.md) を参照。選定理由は **バスの形** であり、「セキュリティが完成した」という意味ではない。

## Protobuf（+ prost）

| | |
|:---|:---|
| **場所** | `protocol/proto/*.proto` → Rust は `prost` / `prost-build` |
| **宣言** | `prost = "0.14"`（固定 ≈ `0.14.4`）。`protoc-bin-vendored` でシステム protoc なしでもビルド可能 |
| **理由** | 言語非依存で版管理しやすい **ワイヤ SSoT**。`PROTOCOL_PIN` で厳密固定。30 Hz フレーム向けに JSON よりコンパクト。スキーマ進化と不一致検出がしやすい。 |
| **明示的に選ばないもの** | ホットパスのピアペイロードとしての JSON（HTTP API・設定には JSON を使う）。 |

Elixir 側の制御面は HTTPS / JWT。空間ピアのペイロードは Zenoh 上の Protobuf。

## 隣接する基盤（短く）

| 技術 | 役割 | 理由（一行） |
|:---|:---|:---|
| **Phoenix 1.8 + Bandit** | auth / world / assets の HTTP | 薄い API。HTTP サーバは Bandit |
| **Ash**（auth / world） | リソースとポリシー | ドメインモデルと認可寄りの API を散らさない |
| **Ecto + PostgreSQL**（auth） | アイデンティティ SSoT | 多ユーザーの永続アカウント |
| **Ecto + SQLite**（world / assets） | Stage-1 台帳 | 単一ノード運用。後の Postgres への伸びしろを残す |
| **Cloudflare Tunnel / R2** | HTTPS 入口 / CAS 選択肢 | 拠点 IP 秘匿。R2 は下り課金に強いオブジェクト保管 |
| **Rapier3d** | host 物理 | Pure Rust の剛体。内部 60 Hz / 配信 30 Hz の分離 |
| **wgpu + naga + winit** | client 描画・窓 | クロスプラットフォーム GPU。naga は将来のシェーダ予算検査にも使う |
| **OpenXR（+ ash）** | VR ランタイム | フルエンジンを持たずにネイティブ VR |
| **BLAKE3** | CAS / seal ハッシュ | アセットと Manifest seal 本体の高速な内容アドレス |
| **Ed25519（dalek）** | Intent / Frame / host 鍵 | ワイヤ上の短いホスト／セッション署名 |
| **Argon2 / Joken+JOSE** | パスワード / JWT | Elixir 側の標準的な認証暗号 |

## この文書が保証しないこと

- 完全な SBOM やライセンス一覧（`cargo deny` 等は別経路）
- [architecture/](../architecture/) の保証表の代替にはならない
