# Alchemy — プラス点統合一覧（まとめ / 2026-11-05）

> 根拠: [Opus](opus/opus-specific-strengths-2026-11-05.md) · [GPT](gpt/gpt-specific-strengths-2026-11-05.md)  
> 採用ルール: 双方が検証した境界を優先。Opus の継続再加点は合意コアに圧縮。今回閉じた契約は明示加点。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている |
| +2 | 業界ベストプラクティスに沿う |
| +3 | 同規模平均を明確に上回る |
| +4 | プロダクション級 OSS と遜色ない |
| +5 | 個人プロジェクトでは稀な卓越 |

| 採用合計（概算） | **~+118** |

---

## 運営側コア

### auth-server

- **アイデンティティ SSoT・Refresh ファミリー・多軸 RL・`:persistent_term` 鍵** `+8` 帯
  > 双方高評価。Keys 直列化解消を含む。減点ほぼなしの最強部品。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`, `auth-server/lib/auth_web/`

### assets-server

- **CAS 物理から access を除去し Grant / PlaceRead に権利を移した** `+4`
  > 前回 P0-5 をカラム削除で解決。同一 hash の cross-owner 共存と認可を両立。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **Quota 予約→CAS→finalize と quarantine GC** `+3`
  > TOCTOU 安全な orphan GC / reserved reaper。
  > 対象ファイル: `assets-server/lib/assets/cas/orphan_gc.ex`

- **PlaceRead / 検疫 Behaviour / Port BLAKE3** `+3` 帯
  > 見る権利の seal 束縛、ライセンス境界、NIF 回避 Port。

### world-server

- **canonical ManifestSeal（中央定義から署名）** `+4`
  > host 提出 hash を信じない。双方の白眉継続。
  > 対象ファイル: `world-server/lib/world/canonical_manifest.ex`

- **CSR mTLS・NotAfter=TTL・CN 強制** `+3`
  > 前回 24h 証明書を正しい境界で閉じた。
  > 対象ファイル: `world-server/lib/world/mtls_issuer.ex`

- **Room RL + 所有上限・PlaceRead 分離** `+2`
  > auth-JWT scope は閉じた（host-token scope は弱み側）。

---

## 共通契約・ホスト実行

### protocol

- **decode 上限・gen/rev・Room AEAD・key helper の共有純関数** `+4`
  > ワイヤ契約 SSoT。AAD に room_id/kind。67 tests。
  > 対象ファイル: `protocol/src/wire_aead.rs`, `protocol/src/decode_limits.rs`

### client/host-server

- **session 署名 Intent・worker 分離・cnf.spk・revision 単調適用** `+4`
  > Untrusted host の入力門番は成熟。inventory の DB 原子性は弱み側。
  > 対象ファイル: `client/host-server/src/room_token/`, `client/host-server/src/network_worker.rs`

### client/app

- **GUI と trust-probe が同一 trust gate** `+4`
  > 前回 adversarial 未通を閉じた。discard 状態機械 + WAN 他室 ciphertext 否定系。
  > 対象ファイル: `client/trust/src/`, `client/app/src/bus/manifest.rs`

- **Desktop/XR hot switch・wgpu 共有** `+3`
  > x0053 実装。surface に閉じる。

### launcher

- **discovery による restart 後 IPC 更新** `+3`
  > 前回 stale を閉じた。8 tests。
  > 対象ファイル: `launcher/src/discovery.rs`

---

## 転送・エッジ・横断

### relay / edge

- **短命 mTLS・bounded Agent・WAN 否定系・CI 入り** `+4`
  > 前回最弱ブロックが水準に接近。部屋隔離は AEAD 本対応へ移行し保証表を自ら修正。
  > 対象ファイル: `relay-agent/src/lib.rs`, `relay-server/config.wan.json5`

### 横断

- **unit / fuzz / hermetic / WAN negative の層・metrics+jti・評価駆動サイクル** `+8` 帯
  > 自己改善が前回 P0 を 1 週間で閉じた点が双方合意の最大の強み。
  > 対象ファイル: `.github/workflows/`, `.workspace/0_doc/evaluation/`

- **4 階層ライセンス境界の明示** `+2`
  > quarantine / verifier 注入点を OSS に閉じ、Proprietary を混入させない。

---

## スコア感（採用プラス）

| 帯 | 概算 |
|:---|---:|
| コア（auth/assets/world） | ≈ +35 |
| protocol / host / client / launcher | ≈ +40 |
| relay / 横断 / プロセス | ≈ +43 |
| **合計概算** | **≈ +118** |

前回採用プラス ≈ +105 から上昇。主因は Relay/CAS/trust E2E の閉鎖と inventory / AEAD の到達。
