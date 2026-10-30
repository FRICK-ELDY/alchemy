# Alchemy — 提案統合一覧（まとめ / 2026-10-28）

> 根拠: [Opus](opus/opus-specific-proposals-2026-10-28.md) · [GPT](gpt/gpt-specific-proposals-2026-10-28.md)  
> すべて採点 `0`。マイナス点の解消策は improvement-plan に落とし、ここは「あると尚よい」を整理する。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 未実装・評価対象外だが、実装すれば価値が上がる提案 |

---

## セキュリティ境界（双方合意コア）

- **検証失敗は破棄、退室は検証済み世代切替のみ** `0`（Opus P0）
  > 偽 Manifest leave DoS の直接対策。否定系 E2E を同梱。

- **world が canonical Manifest を構築・署名（署名 oracle 廃止）** `0`（GPT）
  > DB scene + attested host key + PIN から deterministic body/hash。host は受け取り publish のみ。

- **host generation + monotonic revision gate** `0`（GPT）
  > Manifest/Presence replay 拒否。handoff 時のみ world の新 generation 証明で切替。

- **message class ごとの pre-decode byte / post-decode count limit** `0`（GPT）
  > router max payload・per-peer token bucket と併用。

- **Room Token に `cnf`（session_pub サムプリント）** `0`（Opus）
  > bind 盗聴後勝ちの乗っ取り封じ。DPoP 思想。

- **CasObject / BlobRef / BlobGrant 三層化** `0`（GPT・前回継続）
  > GET 認可 + 補償ジョブ。

- **mTLS + room/tenant ACL + Relay Allocation** `0`（双方）
  > 署名契約の後段。CCU/帯域 QoS。

---

## 実行・体感

- **physics と network/credential worker を bounded channel で分離** `0`（双方）
- **per-session Intent rate limit + JWKS hard stale TTL** `0`（GPT）
- **Grab 保持中 aim の距離クランプ** `0`（Opus）
- **error placeholder（fixture fallback は debug 限定）・jitter buffer / 補間** `0`（GPT）
- **launcher 安定 discovery endpoint（host restart 後 IPC 更新）** `0`（双方）
- **XR ↔ Desktop ホットスイッチ（HMD presence / 明示トグル）** `0`
  > Resonite 相当。起動固定ではなく同一セッションで切替。調査・保証たたきは [x0053](../../roadmap/slices/x0053-xr-desktop-hotswitch.md)。VRC には無い体感差。

---

## 品質・運用・配布

- **world テスト DB 隔離（assets と同型）・prod compose 起動 smoke** `0`（Opus — マイナス直結だが提案としても固定）
- **adversarial E2E + cargo-fuzz / StreamData** `0`（双方）
- **OpenTelemetry / Prometheus SLO** `0`（双方）
- **launcher を CI に・SBOM / 署名 / installer** `0`（双方）
- **`/internal/*` ingress 404・Social レート制限・mix_audit/sobelow** `0`（Opus）
- **BLAKE3 ストリーミング NIF・auth Keys の persistent_term・ClientIp 共有化** `0`（Opus）

---

## 目標のみ（減点しない・評価対象外）

- **verifier（Spot-Check）・Formula VM・Wake-on-Join・OSC・Dual Editor・LOD/KTX2・空間オーディオ**
- **billing / market-server / FederationPolicy**
  > いずれもディレクトリ未設置または契約未。world-authored Manifest・generation・BlobGrant が先行条件。

---

## 優先の読み方

マイナス直結の P0 は improvement-plan に移した。ここでの最優先提案は「canonical Manifest + revision gate + decode limit + leave→discard」のセット。これが揃うと verifier / market の前提が初めて立つ。
