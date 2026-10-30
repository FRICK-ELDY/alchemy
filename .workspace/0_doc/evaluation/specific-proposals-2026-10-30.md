# Alchemy — 提案（0点）統合一覧（まとめ / 2026-10-30）

> 根拠: [Opus](opus/opus-specific-proposals-2026-10-30.md) · [GPT](gpt/gpt-specific-proposals-2026-10-30.md)  
> 加点も減点もしない。目標のみ・未実装。新規依存は **人に回す**。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 実装すれば価値が上がる提案 |

---

## 転送・公開エッジ（最優先帯と隣接）

- **payload の Room 鍵 AEAD（キー式隔離に依存しない）** `0`
  > wildcard subscribe でも他部屋内容を読めなくする。依存追加は人に回す。参考: Matrix Megolm。

- **Relay L7 化（部屋単位の購読強制）** `0`
  > Allocation の `room_id` / `wire_cap` 以外の宣言を拒否。

- **client CSR + 秒単位 NotAfter + hard JWKS / issuance bound** `0`
  > 秘密鍵をネットワークに出さず、証明書寿命と `expires_in` を一致させ、資源上限を一契約に。

- **WAN 構成の否定系 E2E（mTLS + ACL + relay）** `0`
  > guest の wildcard subscribe / put 拒否を CI で固定。

- **QoS / CCU の固定 free-tier 計測** `0`
  > billing 未実装でも allocation 単位 budget を検証可能に。

---

## 運営側コア

- **access を Grant/ref 側へ移し GET を用途 capability で統一（案 B）** `0`
- **予約 reaper・orphan CAS GC・streaming hash** `0`
- **world rate limit の shared 化 + Room 上限** `0`
- **canonical Manifest bytes を world が返す** `0`
- **verifier Spot-Check** `0`（canonical seal / gen-rev の先行条件は満たされた）
- **鍵ローテーション演習の定期化** `0`

---

## 契約・実行・端末

- **inventory ladder（x0054〜x0065）を文書順で実装** `0`
  > Grant PUT → held slot → take/put → rejoin → Drive/VCS/Origin。部屋隔離を先に閉じると機密性が乗る。
- **user_root 幅を Manifest 配下へ** `0`
- **予測・補間バッファ（verified state と分離）** `0`
- **XR↔Desktop hot switch（x0053）** `0`
- **Formula VM / Wake-on-Join / OSC / LOD / audio** `0`
- **launcher 固定 discovery（IPC stale・deep link・handoff の control plane）** `0`

---

## 横断

- **headless client trust consumer を Compose E2E へ** `0`
- **XR 実 GPU nightly 煙** `0`
- **BLAKE3 / seal-hash の常駐 Port** `0`
- **OTLP / 相関 ID（Room Token `jti`）** `0`
- **インストーラ・SBOM・artifact 署名** `0`
- **billing / market / Optional Profile** `0`（ディレクトリ無し・対象外継続）
