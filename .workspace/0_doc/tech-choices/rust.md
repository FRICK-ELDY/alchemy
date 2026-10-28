# Rust クレート選定

出典: [`client/Cargo.toml`](../../../client/Cargo.toml)（ワークスペース）、[`protocol/Cargo.toml`](../../../protocol/Cargo.toml)、[`launcher/Cargo.toml`](../../../launcher/Cargo.toml)。固定版の例は `client/Cargo.lock`（2026-10-28 時点）。

## ワークスペース構成

| クレート | パス | 役割 |
|:---|:---|:---|
| `alchemy-protocol` | `protocol/` | Protobuf ワイヤ SSoT + 暗号ヘルパ |
| `alchemy-shared-sim` | `client/shared-sim/` | 共有 EntityKind / ManifestIndex（Rapier なし） |
| `host-server` | `client/host-server/` | ローカル空間権威ホスト |
| `client` | `client/app/` | Desktop / OpenXR エンドポイント |
| `zenoh-connect` | `client/zenoh-connect/` | Zenoh 接続と TLS ルート CA の共有 |
| `e2e-probe` | `client/e2e-probe/` | Hermetic Compose E2E（GUI なし） |
| `alchemy-launcher` | `launcher/` | host+client 起動、localhost IPC で停止 |

## コア / ワイヤ

| クレート | 宣言 | 固定（目安） | 利用者 | 理由 |
|:---|:---|:---|:---|:---|
| `zenoh` | `1.10` | `1.10.1` | host, client, connect, e2e | ピア転送（[stack.md](stack.md)） |
| `prost` | `0.14` | `0.14.4` | protocol, host, client | Protobuf の encode / decode |
| `prost-build` | `0.14` | — | protocol（build） | ビルド時に `.proto` をコンパイル |
| `protoc-bin-vendored` | `3` | — | protocol（build） | システム `protoc` なしでも再現ビルド |
| `ed25519-dalek` | `2` | `2.2.0` | protocol | Intent / host / seal まわりの署名 |
| `blake3` | `1` | `1.8.7` | protocol, client | CAS キー、Manifest 本体ハッシュ |
| `serde` / `serde_json` | `1` | — | 多数 | 設定、HTTP JSON、接続設定 JSON |
| `hex` | `0.4` | — | protocol | ツール／テストでのハッシュ hex |
| `rand` | `0.8` | — | protocol（+ host テスト） | 鍵・nonce 生成 |

## シミュレーション / 数学

| クレート | 宣言 | 固定（目安） | 利用者 | 理由 |
|:---|:---|:---|:---|:---|
| `rapier3d` | `0.22` | `0.22.0` | host-server | 60 Hz の剛体物理 |
| `glam` | `0.29` | `0.29.3` | client | カメラ／UI 用の CPU 側ベクトル・行列 |

## グラフィックス / XR / 窓

| クレート | 宣言 | 固定（目安） | 利用者 | 理由 |
|:---|:---|:---|:---|:---|
| `wgpu` | `24` | `24.0.5` | client | クロスバックエンド GPU |
| `naga` | `24`（wgsl-in, spv-out） | `24.0.0` | client | シェーダ IR。wgpu 24 と揃え、将来のコスト検査にも使う |
| `winit` | `0.30` | `0.30.13` | client | Desktop の窓と入力 |
| `openxr` | `0.22` | `0.22.0` | client | VR セッション／スワップチェーン |
| `ash` | `0.38` | — | client | OpenXR との Vulkan 連携 |
| `gltf` | `1` | — | client | CAS メッシュの読み込み |
| `bytemuck` | `1`（+derive） | — | client | GPU 向け `Pod` / `Zeroable` キャスト |
| `png` | `0.17` | — | client | UI／テクスチャ用の画像デコード |
| `pollster` | `0.4` | — | client | メインスレッドで wgpu 初期化を待つ |

## HTTP / JWT（Rust 側）

| クレート | 宣言 | 利用者 | 理由 |
|:---|:---|:---|:---|
| `ureq` | `2`（+json） | host, client, e2e | auth / world / assets への同期 HTTP（host ループに重い async ランタイムを載せない） |
| `jsonwebtoken` | `9` | host, client | JWKS 経由の Room Token 等の検証 |
| `base64` | `0.22` | host, client | JWT／鍵材料の符号化 |

## プロセス / 運用 / その他

| クレート | 宣言 | 利用者 | 理由 |
|:---|:---|:---|:---|
| `anyhow` | `1` | 多くの bin | プロセス境界での文脈付きエラー |
| `log` + `env_logger` | `0.4` / `0.11` | bin | 簡潔な stderr ログ |
| `ctrlc` | `3` | launcher | Ctrl-C の優雅な処理 |
| `getrandom` | `0.2` | launcher | IPC トークン用乱数 |
| `libc` | `0.2` | client | OS 低レベル呼び出し |
| `winreg` | `0.52` | client（Windows） | Steam / OpenXR ローダ探索 |

### 開発専用（host-server テスト）

| クレート | 宣言 | 理由 |
|:---|:---|:---|
| `rsa` / `pkcs1` / `pkcs8` | `0.9` / `0.7` / `0.10` | 本番 auth なしでテスト用 JWT／鍵を発行 |

## ホットパスに載せないもの

- host-server に Tokio 中心スタックを載せない（物理ループを重い async ランタイムから守る）
- `protocol` を client ワークスペースのソースとして埋め込まない。リポジトリ直下への path 依存でワイヤクレートを移しやすくする
