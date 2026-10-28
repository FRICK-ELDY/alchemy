# Alchemy — 総合評価レポート（まとめ / 2026-10-26）

- **評価日**: 2026-10-26
- **種別**: 初回評価（前回まとめ・アーカイブなし）
- **評価対象**: `main`（双方とも当日コード直接検証。Opus 記録コミット `683e30f`）
- **第1評価者**: [Claude Opus 5.5](opus/opus-evaluation-2026-10-26.md) — **+96 / -72 = +24**
- **第2評価者**: [GPT-5.6 Sol](gpt/gpt-evaluation-2026-10-26.md) — **+61 / -62 = -1**
- **詳細統合**: [強み](specific-strengths-2026-10-26.md) · [弱み](specific-weaknesses-2026-10-26.md) · [提案](specific-proposals-2026-10-26.md) · [改善計画](improvement-plan-2026-10-26.md)

---

## ひとことで言うと

**部品（auth・protocol・Presence・surface）は同規模個人プロジェクトの平均を大きく上回るが、x001a/x001b で「外に出した」瞬間の信頼境界が開いたままである。**  
設計の方向性は正しい。現状を WAN 参加可能な分散 VR 基盤として扱うのは危険。

---

## スコアの扱い（合意・相違・採用）

| | プラス | マイナス | 正味 |
|:---|---:|---:|---:|
| 第1（Opus） | +96 | -72 | **+24** |
| 第2（GPT） | +61 | -62 | **-1** |
| **まとめ採用（概算）** | **~+75** | **~-72** | **~+3** |

### なぜ差が出たか

| 軸 | Opus | GPT | まとめの判断 |
|:---|:---|:---|:---|
| プラスの幅 | 文書規律・XR 細部・auth まで広く加点 | 検証できた境界に絞る | 合意コア + 文書規律（+4）を採用。Opus の広い加点は参考 |
| マイナスの厳しさ | Tunnel dev 公開・Token 期限・JWKS 同期・なりすましを厚く | host_control・assets GET・host role・平文 WAN・client 無検証を厚く | **両方採用**（重なりは折衷、片方発見は採用） |
| 姿勢 | 「部品は優秀、境界が開いている」 | 「方向は良い、配布可能性は低い」 | **両立** — 部品 +、境界 − |

採用正味 **約 +3** は、「内部設計はプラスだが、公開前提のセキュリティでほぼ相殺」という意味。楽観（+24）でも悲観（-1）でもなく、**境界を塞げばスコアは急上昇する**と読む。

---

## ローカル CI（双方一致）

| ターゲット | 結果 |
|:---|:---|
| rust（client 55 + host 19） | PASS |
| protocol（7） | PASS（protoc PATH 無し時は compile SKIP） |
| world（6） | PASS |
| auth（107） | PASS |
| assets（7） | **FAIL 1**（`.env` → runtime が test を S3 化し 302） |

依存: hackney HIGH/MEDIUM advisory を assets 実行時に確認。smoke 縦通し・実機 HMD は未確認（双方）。

---

## ブロック別講評（統合）

| ブロック | 採用読み | 合意 |
|:---|:---|:---|
| auth-server | 最強部品。refresh race・Tunnel IP が減点 | 双方高評価 |
| assets-server | CAS/302 は良い。認可・Quota race・Parsers・CI 隔離が弱点 | GET 認可の重みは相違→折衷 `-3` |
| world-server | claim 設計は良い。role 自己申告が門番を空洞化 | 折衷 `-3` |
| protocol | SSoT・チャネル分離は優秀。PIN 非ワイヤ・回転欠如・room_id 不一致 | 合意 |
| host-server | Tick/Presence/ゲートは良い。shutdown・magnitude・JWKS 同期が重大 | 重大項は双方または片方採用 |
| client/app | surface/Facet/BLAKE3 は良い。Token 非更新・無検証 Frame・blocking UI | 双方 |
| launcher | 対象外 | 合意 |
| zenoh/edge | 設定・手順は良い。dev Tunnel・平文 WAN が価値命題を毀損 | 双方（観点の切り方は異なる） |
| 横断 | 単体は厚い。E2E・監査・可観測性が薄い。定義 vs 実行の逆転 | 合意 |

---

## 相違の明示と採用判断

| 項目 | Opus | GPT | 採用 |
|:---|:---|:---|:---|
| host_control 無認証 | -2 | -5 | **-5**（実コードが exit） |
| Tunnel dev / mailbox | -4 | （未個別） | **-4** |
| WAN 平文 | -2 | -4 | **-4** |
| assets GET 無所有者検査 | -1 | -4 | **-3** |
| host role 自己申告 | -2 | -4 | **-3** |
| 既定 alice/Password1 | -1 | -4 | **-3** |
| Room Token 非更新 | -3 | 未 | **-3** |
| JWKS 入力スレッド同期 | -3 | 未 | **-3** |
| liveliness なりすまし | -3 | 未 | **-3** |
| Token 盗聴なりすまし | -3 | 連鎖で指摘 | **-3** |
| client Frame 無検証 | 弱 | -4 | **-3** |
| 定義 vs 実行逆転 | -3 | entity 重複 -2 | **-3** |
| architecture 文書規律 | +4 | +2 帯 | **+4** |

独立評価として、**相手が見落とした重大穴を相互に補完した**のが今回の価値である。

---

## 同種 OSS 比較（統合）

| 軸 | 比較 | 所見 |
|:---|:---|:---|
| 認証 | Phoenix gen.auth / Keycloak | refresh 再利用検知・多軸 RL は gen.auth 超。Friendship/OIDC は未 |
| 空間同期 | Resonite / VRChat | Manifest×姿勢分離は同筋。回転・補間・ACL・スナップショットは商用初日から必須級が欠落 |
| Zenoh | 公式デプロイ例 | TLS/ACL/QoS 未使用。強みの半分 |
| エンジン | Bevy + OpenXR | XR 生 Vulkan 二重経路は保守負債 |
| 文書 | 個人 OSS | 保証表の3層一致は稀少 |

---

## 次にやるべきこと（トップ5）

詳細は [improvement-plan-2026-10-26.md](improvement-plan-2026-10-26.md)。

1. **公開経路を release のみに**（Tunnel + `/dev/*` 封鎖）— コスト最小・効果最大
2. **host_control 無効化 or ローカル IPC 化**、既定シードを release で fail-closed
3. **Room Token 自動更新 + JWKS バックグラウンド化**
4. **Zenoh 身元保護**（liveliness 紐付け、Token バス載せ見直し、WAN TLS/ACL）
5. **assets CI 隔離 + GET 認可方針の文書化または grant 実装 + world host role 制限**

---

## 評価の前提

- 旧 Elixir 権威実行前提は未使用
- 目標のみの項目は減点せず提案/対象外
- まとめは双方の詳細を読み、合意・相違・採用を明示した（各評価者は相手文書を読んでいない）
