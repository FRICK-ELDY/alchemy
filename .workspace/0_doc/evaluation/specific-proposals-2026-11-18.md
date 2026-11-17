# Alchemy — 提案統合一覧（まとめ / 2026-11-18）

> 根拠: [Opus](opus/opus-specific-proposals-2026-11-18.md) · [GPT](gpt/gpt-specific-proposals-2026-11-18.md)  
> 提案は加点も減点もしない（`0`）。弱点の解消手順は [improvement-plan](improvement-plan-2026-11-18.md) 側。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では存在しないが、実装すれば価値が上がる提案 |

---

## 運営側コア

- **durable relation outbox + 影響 owner 再評価** `0`（双方）
  > social mutation と同一境界に outbox。`friends_plus` / `followers_plus` は二者固定走査ではなく影響集合を再計算。
- **Permissions UI（Editors / Eligible / Denied）** `0`（Opus）
  > HostDelegation 表の HTTP/UI 管理。kick 拒否リストと同画面。
- **署名付き HostDelegation JWT** `0`（Opus）
  > host が world ダウン時も委譲内容を自己検証。
- **Room 鍵の push 型 epoch ヒント** `0`（Opus）
  > 軽量ポーリング残りの置き換え。
- **verifier Spot-Check オーケストレーション（OSS は配線のみ）** `0`（双方）
  > 統計・閾値は Proprietary。`sim_state` が入力下地。
- **Origin Cert 鍵履歴 / 失効 / clone-fetch** `0`（双方）
  > 連合前の必須。Spatial VCS の次段。

---

## シミュレーション・配信

- **既定経路の確保回数・4096 時間予算ベンチ** `0`（双方）
  > MVCC 証明を Epoch/host 本線へ。切替ゲートを CI に固定。
- **LBVH Karras / 複数 HotPhysicsPage** `0`（双方）
  > SceneBudget 4096 と容量を一致させる拡張。
- **AOI 単位 snapshot・delivery 状態機械の property test** `0`（双方）
  > cell 本線の収束保証。
- **`sim_state` を planned handover に配線 / GPU 補間ディスパッチ** `0`（Opus）
  > 部品はあり、起動・描画経路の接続が残る。
- **host dual-publish 停止（Room frame → cell/lod のみ）** `0`（Opus）

---

## 配布・運用・目標

- **署名付き Windows bundle + SBOM/provenance** `0`（双方）
  > OS vault 導入後、未署名バイナリの資格情報アクセスがより気になる。
- **本番 SLO dashboard・長時間 soak** `0`（GPT）
- **S2S 連合 / Dual-AST / Wake-on-Join / OSC / LOD ビルド / 空間オーディオ / billing·market** `0`
  > 目標。ディレクトリ未設置・未実装は減点しない。
