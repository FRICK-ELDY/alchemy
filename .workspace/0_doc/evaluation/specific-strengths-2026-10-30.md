# Alchemy — プラス点統合一覧（まとめ / 2026-10-30）

> 根拠: [Opus](opus/opus-specific-strengths-2026-10-30.md) · [GPT](gpt/gpt-specific-strengths-2026-10-30.md)  
> 採用ルール: 双方が検証した境界・設計判断を優先。Opus の細分割加点はまとめでは束ねる。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている |
| +2 | 一般的なベストプラクティスに沿う良い設計 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る |
| +4 | プロダクション級 OSS と比較しても遜色ない |
| +5 | 個人プロジェクトとして卓越している |

| 採用合計（概算） | **~+105** |

---

## 運営側コア

### auth-server

- **アイデンティティ・refresh family・JWKS・Social SSoT** `+4`
  > Argon2、rotation+reuse detection、kid ローテーション、Follow/Friendship の DB 制約、S2S predicate、Tunnel IP、Social RL、prod fail-closed。Room/空間計算を引き取らない分離も維持。
  > 対象ファイル: `auth-server/lib/auth/`

### assets-server

- **CasObject / BlobRef / BlobGrant / Quota 予約補償** `+4`
  > 物理と参照の分離、予約→TX 外 put→確定、cross-owner dedupe、未知 access fail-closed、`grant_required` GET。access を CasObject に置いた点は弱点側。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

### world-server

- **canonical ManifestSeal（署名 oracle 廃止）** `+4`
  > DB scene・pin・attestation・永続 host generation から hash を再計算。host 提出 hash は照合のみ。
  > 対象ファイル: `world-server/lib/world/canonical_manifest.ex`

- **Room owner / visibility / 用途別 JWT** `+3`
  > owner のみ host、social S2S 障害 503、Token/Attestation/Seal の aud/typ 分離。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

---

## 共通契約層

### protocol

- **decode limit + generation/revision ゲートの共有純関数** `+4`
  > class 別 pre-decode byte / post-decode count。host/client が同一関数を使用。PIN・キー式・role capability も独立 crate。
  > 対象ファイル: `protocol/src/decode_limits.rs`, `protocol/src/generation_gate.rs`

---

## ホストユーザー端末

### client/host-server

- **Token→cnf→session→seq→rate→finite の多段ゲート** `+4`
  > bind 盗聴だけでは別鍵 bind 不可。Intent rate、JWKS hard stale。
  > 対象ファイル: `client/host-server/src/session.rs`, `intent_rate.rs`

- **credential / network worker 分離（失敗で殺さない）** `+4`
  > 物理 60Hz から同期 HTTP/Zenoh wait を外し、hard expiry 後だけ publish 停止。
  > 対象ファイル: `client/host-server/src/room_token.rs`, `main.rs`

### client/app

- **検証失敗は discard（leave-on-fail 廃止）+ gen/rev** `+4`
  > pin/seal/attest/replay 失敗で部屋を空にしない。明示 Leave のみ退室。
  > 対象ファイル: `client/app/src/main.rs`

- **XR wgpu 統合（vk_loop 削除）** `+4`
  > OpenXR Vulkan を wgpu-hal に adopt、SceneGpu dual-eye。描画経路二重化を解消。
  > 対象ファイル: `client/app/src/surface/xr/`

- **asset placeholder / HTTP timeout** `+2`
  > release で fixture 隠蔽なし。
  > 対象ファイル: `client/app/src/main.rs`, `world_api.rs`

### launcher

- **CI・restart policy 純関数化** `+2`
  > 前回 CI 外・clippy 赤を解消。IPC stale は非保証として残る。
  > 対象ファイル: `launcher/src/`

---

## 転送・エッジ

- **WAN mTLS + role CN ACL + default deny（方向）** `+3`
  > TLS のみから身元付き転送へ前進。実効の部屋隔離・証明書寿命は弱点側。
  > 対象ファイル: `zenoh-router/config.wan.json5`, `relay-agent/`

- **Cloudflare Tunnel による origin 秘匿** `+2`
  > `/internal/*`・`/dev/*` ingress 404。
  > 対象ファイル: `edge/cloudflared/`

---

## 横断

- **テスト基盤の伸長（fuzz・adversarial host・test DB 隔離・全緑 CI）** `+4`
  > cargo-fuzz smoke、world 隔離、launcher CI、prod compose config。relay と client trust E2E は弱点。
- **loopback Prometheus metrics（host/client/assets）** `+2`
- **保証表・threat model・評価駆動スライス運用** `+4`
  > 前回弱点の大半をコードで閉じた自己改善サイクル。
- **Windows ネイティブ日常開発 + Docker Elixir** `+2`

---

## スコア感（採用プラス）

| 帯 | 概算 |
|:---|---:|
| コア（auth/assets/world） | ≈ +18 |
| protocol / host / client | ≈ +22 |
| 転送・launcher | ≈ +7 |
| 横断 | ≈ +12 |
| 継続の細部（双方の +1〜+2 束） | ≈ +46 |
| **合計概算** | **≈ +105** |

Opus +262 は継続項目の細分割が大きい。GPT +86 は検証境界に絞る。まとめは「合意コア＋今回閉じた契約」を中心に採用。
