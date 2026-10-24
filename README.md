# Alchemy

分散連合型 VR プラットフォーム（目標アーキテクチャ: [`.workspace/0_doc/architecture/overview.md`](.workspace/0_doc/architecture/overview.md)）。

## Slice A（現状）

ローカル `zenohd` → `host-server` が RenderFrame を Publish → `client` が Subscribe して表示。  
認証は `auth-server`（JWT / JWKS）。

| コンポーネント | パス |
|:---|:---|
| auth-server | [`auth-server/`](auth-server/) |
| zenoh-router | [`zenoh-router/`](zenoh-router/) |
| protocol | [`crates/protocol/`](crates/protocol/) |
| host-server | [`crates/host-server/`](crates/host-server/) |
| client | [`crates/client/`](crates/client/) |

起動手順は [`.workspace/roadmap.md`](.workspace/roadmap.md)。
