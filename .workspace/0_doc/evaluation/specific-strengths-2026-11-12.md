# Alchemy — プラス点統合一覧（まとめ / 2026-11-12）

> 根拠: [Opus](opus/opus-specific-strengths-2026-11-12.md) · [GPT](gpt/gpt-specific-strengths-2026-11-12.md)  
> 採用ルール: 双方が検証した境界を優先。Opus の継続再加点は合意コアに圧縮。今回閉じた契約は明示加点。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている |
| +2 | 業界ベストプラクティスに沿う |
| +3 | 同規模平均を明確に上回る |
| +4 | プロダクション級 OSS と遜色ない |
| +5 | 個人プロジェクトでは稀な卓越 |

| 採用合計（概算） | **~+132** |

---

## 運営側コア

### auth-server

- **アイデンティティ SSoT・Refresh ファミリー・多軸 RL・fail-closed 鍵・`/ready`** `+8` 帯
  > 双方高評価・減点なしの最強部品。Social / S2S / readiness 分離も安定。
  > 対象ファイル: `auth-server/lib/auth/`, `auth-server/lib/auth_web/`

### assets-server

- **CAS / Quota / Grant・PlaceRead 分離 + content-bound Origin Cert + Port プール / hermetic test / `/ready`** `+12` 帯
  > 前回残件（Port 直列・mime・Origin Cert 非束縛・test flake）を閉じた。Grant モデルは git LFS より権利境界が明確。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`, `assets-server/lib/assets/vcs/cert.ex`, `assets-server/lib/assets/blake3/worker.ex`

### world-server

- **host HTTP PoP + active_host ledger + Scene CAS + Room key epoch + host 操作別 RL + lease CAS** `+14` 帯
  > 前回 P0 の大半を否定系付きで閉じた。門番としての厚みは今回最大の伸び。権限表（takeover）の穴は弱み側。
  > 対象ファイル: `world-server/lib/world/http_pop.ex`, `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/plugs/rate_limit.ex`

---

## 共通契約層

### protocol

- **ワイヤ・AEAD v2・control plane 署名・HTTP PoP canonical・decode 上限の単一正本** `+8` 帯
  > 双方「最も完成度の高い層」で合意。
  > 対象ファイル: `protocol/src/wire_aead.rs`, `protocol/src/control_plane.rs`, `protocol/src/http_pop.rs`

### client/shared-sim

- **SoA Epoch tape・pointer-free topology・XPBD / Flat LBVH の汎用クレート化** `+4`
  > 固有コンテンツを持ち込まない方向は正しい。ホットパス確保は弱み。
  > 対象ファイル: `client/shared-sim/src/`

---

## ホストユーザー端末

### client/host-server

- **Intent 多層 gate + HostLoop / network / credential 分離 + pose プール** `+10` 帯
  > Untrusted bus 前の検査と tick 切り出しは双方高評価。
  > 対象ファイル: `client/host-server/src/session.rs`, `client/host-server/src/host_loop.rs`

### client/app / HostSupervisor

- **共有 trust gate・GLB budget・Desktop/OpenXR hot switch・HostSupervisor（Job Object / PDEATHSIG）** `+12` 帯
  > launcher 退役を中途半端に残さず親プロセスへ移した点を双方評価。資格情報寿命は弱み。
  > 対象ファイル: `client/trust/`, `client/app/src/host_supervisor.rs`, `client/app/src/asset_decode_budget.rs`

---

## 転送・エッジ

- **relay-agent CSR-only + 非 loopback 拒否 + 主体別 throughput** `+5` 帯
  > 前回 `key_pem` 残件を閉じた。
  > 対象ファイル: `relay-agent/src/lib.rs`, `relay-agent/src/throughput.rs`

---

## 横断

- **否定系・WAN 実 host E2E・fuzz・単一 CI 入口・readiness / metrics・評価駆動速度** `+19` 帯
  > 前回 P0〜P2 の大半を 1 週間で閉じた自己改善は継続。今回は新機能（x0080 / x0093）の権限・機能後退を差し引き、過大加点しない。
  > 対象ファイル: `.scripts/ci.sh`, `.workspace/roadmap/slices/x0081-*.md` 〜 `x009c-*.md`

---

## スコア感（採用プラス）

| 帯 | 点数 |
|:---|---:|
| 運営側コア | ≈ +34 |
| 共通契約 | ≈ +12 |
| ホスト端末 | ≈ +22 |
| 転送・エッジ | ≈ +5 |
| 横断 | ≈ +19 |
| 今回閉じた契約の明示加点 | ≈ +40 |
| **合計概算** | **≈ +132** |

前回まとめ採用プラス ≈ +118 から上昇。主因は PoP / epoch / scene CAS / CSR-only / HostSupervisor / shared-sim の契約閉鎖。
