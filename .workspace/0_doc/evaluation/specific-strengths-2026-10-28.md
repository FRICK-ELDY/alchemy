# Alchemy — プラス点統合一覧（まとめ / 2026-10-28）

> 根拠: [Opus](opus/opus-specific-strengths-2026-10-28.md) · [GPT](gpt/gpt-specific-strengths-2026-10-28.md)  
> 採用方針: 合意コアを採用。Opus の細分化加点は参考とし、GPT の検証できた境界加点をアンカーにする。卓越項目のみ +4。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている |
| +2 | 一般的なベストプラクティスに沿う良い設計 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る |
| +4 | プロダクション級 OSS と比較しても遜色ない |
| +5 | 個人プロジェクトとして卓越している |

| 採用合計（概算） | **~+92** |

---

## 運営側コア

### auth-server

- **アカウント・JWT・refresh family を一つの権威に集約** `+4`（双方）
  > Argon2、RS256/kid/JWKS、password lifecycle、deactivate。他サービスは `sub` のみ。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

- **Refresh の FOR UPDATE + 条件付き revoke** `+3`（双方・前回弱点解決）
  > 二重 rotation を実装で閉じ、競合契約テストあり。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

- **Follow / Friendship + S2S access predicate** `+3`（双方）
  > 関係グラフを JWT に肥大化させず、DB CHECK で不変条件を守る。
  > 対象ファイル: `auth-server/lib/auth/social.ex`

- **多軸レート制限・ClientIp（CF-Connecting-IP）・情報を漏らさない Plug** `+2`
  > 対象ファイル: `auth-server/lib/auth/rate_limit.ex`, `auth-server/lib/auth_web/client_ip.ex`

### assets-server

- **CAS（BLAKE3）+ local/S3 + 302 + 条件付き Quota** `+3`（双方）
  > 前回の race / test env 汚染は解決。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`, `assets-server/lib/assets/cas.ex`

### world-server

- **永続 Room owner + host role 強制 + visibility fail-closed** `+3`（双方）
  > 暗黙 create 禁止、storage failure→503、S2S 障害は許可に潰さない。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`, `world-server/lib/world/room_access.ex`

- **Room Token / HostAttestation / ManifestSeal の用途分離** `+2`（双方）
  > typ/aud/role 分離。guest から HTTP capability 拒否のテストあり。seal の内容保証は弱点側。
  > 対象ファイル: `world-server/lib/world/host_attestation.ex`, `world-server/lib/world/manifest_seal.ex`

---

## 共通契約・ホスト端末

### protocol

- **独立 crate・PIN・TRS・wire_crypto・delivery 意味論** `+4`（双方・前回弱点群の解消）
  > PIN 搭載、TRS、共有 room validator、tick gap→snapshot、Ed25519 署名規約の集約。
  > 対象ファイル: `protocol/src/lib.rs`, `protocol/src/wire_crypto.rs`, `protocol/peer-delivery.md`

### client/host-server

- **Token→session sig→seq→capability→sanitization** `+3`（双方）
  > 巨大 move / 遠距離 Grab 回帰テスト。
  > 対象ファイル: `client/host-server/src/session.rs`

- **Rapier 60Hz / delta 30Hz / snapshot・Presence reclaim・credential refresh** `+3`（双方）
  > liveliness jti 束縛、fatal poison。物理ループ内同期 HTTP は弱点。
  > 対象ファイル: `client/host-server/src/main.rs`

### client/app

- **Frame 署名 / finite / tick / snapshot 収束** `+3`（双方・前回偽 Frame 解決）
  > 対象ファイル: `client/app/src/main.rs`

- **Manifest seal + HostAttestation・Token 80% 自動更新・UI background job** `+3`（双方）
  > leave-on-fail は弱点。
  > 対象ファイル: `client/app/src/main.rs`, `client/app/src/manifest_seal.rs`

- **Manifest / shared-sim 駆動 TRS** `+2`
  > 定義 vs 実行の逆転は大部分解消。
  > 対象ファイル: `client/shared-sim/src/lib.rs`, `client/app/src/scene.rs`

### launcher

- **spawn / ready / loopback IPC / monitor / backoff の最小実装** `+2`（双方）
  > Zenoh から process-control を分離。
  > 対象ファイル: `launcher/src/main.rs`

### zenoh / edge

- **WAN TLS + Tunnel release / `/dev/*` deny** `+3`（双方・前回 -4 解決）
  > 対象ファイル: `zenoh-router/config.wan.json5`, `edge/cloudflared/config.yml.example`

---

## 横断

- **Hermetic Compose E2E + 境界回帰テスト増** `+3`（双方）
  > Rust workspace・protocol・auth・assets はローカル緑。adversarial は未。

- **hex.audit / cargo-deny gate・alchemy_auth_client・undocumented_unsafe deny** `+3`（双方）
  > 前回衛生指摘への仕組み応答。

- **architecture / trust-boundary とコードの一致・自己改善サイクル** `+4`（双方・Opus 強調）
  > 評価→改善計画→スライス→PR が 2 日で 23 本回った事実は稀少。
  > 対象ファイル: `.workspace/0_doc/architecture/`, `.workspace/roadmap/slices/`

- **storage/auth failure の fail-closed 分類** `+2`
- **Docker/WSL/個別 CI 導線** `+2`

---

## 採用プラスの読み方

| | Opus | GPT | まとめ |
|:---|---:|---:|---:|
| プラス合計 | +176 | +69 | **~+92** |

Opus は細項目に分割して加点が膨らむ。GPT は検証境界に絞る。まとめは「合意した卓越・平均超え」を中心に積み、文書規律と自己改善サイクルの +4 を採用。
