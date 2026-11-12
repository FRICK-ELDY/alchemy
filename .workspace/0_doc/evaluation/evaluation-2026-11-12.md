# Alchemy — 総合評価レポート（まとめ / 2026-11-12）

- **評価日**: 2026-11-12
- **種別**: 5回目（前回まとめ: [archive/2026-11-05](archive/2026-11-05/evaluation-2026-11-05.md)）
- **評価対象**: `main` @ `8833821`（PR #187 / x009c 直後）。前回 `c491016` から **122 コミット**（x007c〜x009c、launcher 退役、HostSupervisor、Room key epoch、制御面 PoP、scene CAS、Zero-Base spatial / XPBD）
- **第1評価者**: [Claude Opus 5.5](opus/opus-evaluation-2026-11-12.md) — **+388 / -19 = +369**
- **第2評価者**: [GPT-5.6 Sol](gpt/gpt-evaluation-2026-11-12.md) — **+82 / -19 = +63**
- **詳細統合**: [強み](specific-strengths-2026-11-12.md) · [弱み](specific-weaknesses-2026-11-12.md) · [提案](specific-proposals-2026-11-12.md) · [改善計画](improvement-plan-2026-11-12.md)

---

## ひとことで言うと

**前回の P0（PoP・鍵寿命・scene CAS・CSR-only）は 1 週間でほぼ閉じた。次の穴は、新しく入れた host 移譲の権限表と、通常 UI ホストの資格情報寿命である。**

---

## スコアの扱い（合意・相違・採用）

| | プラス | マイナス | 正味 |
|:---|---:|---:|---:|
| 第1（Opus） | +388 | -19 | **+369** |
| 第2（GPT） | +82 | -19 | **+63** |
| **まとめ採用（概算）** | **~+132** | **~-23** | **~+109** |
| 前回まとめ採用 | ~+118 | ~-24 | **~+94** |

### なぜ差が出たか

| 軸 | Opus | GPT | まとめの判断 |
|:---|:---|:---|:---|
| プラスの幅 | 継続項目を細かく再加点（+388） | 検証できた境界に絞る（+82） | 合意コア＋今回閉じた契約。主指標は**旧 P0 閉鎖と新穴の採用** |
| マイナスの焦点 | takeover→scene 改ざん・XPBD 後退・UI JWT・dash 肥大 | 無委譲 takeover -4・資格情報 -3・hot-path alloc・handover | **両方採用（厳しめ側）** |
| 姿勢 | 「門番は厚くなった。定義↔実行が新機能で崩れた」 | 「閉じた契約は認める。移譲面を過大加点しない」 | **両立** |

採用正味 **約 +109**（前回約 +94）は、「旧 P0 を閉じた伸び」から「takeover / UI host 資格情報の減点」を差し引いた結果。

---

## ローカル CI（双方一致寄り）

| ターゲット | Opus | GPT | まとめ |
|:---|:---|:---|:---|
| client workspace | PASS（447） | PASS（447） | **PASS** |
| protocol | PASS（73） | PASS（73） | **PASS** |
| relay-agent | PASS（32） | PASS（32） | **PASS** |
| auth | PASS（133） | PASS（133）※audit は registry timeout で省略経路あり | **PASS** |
| world | PASS（162） | PASS（162）※初回 FS error→再実行 | **PASS** |
| assets | 初回 credo クラッシュ → 再実行 PASS（191） | 一度で PASS（191） | **PASS（credo フレーク記録）** |
| shared/alchemy_* | PASS（25）※Git Bash 経路 | 環境未完了（Alpine 無出力停止） | **PASS（一方経路）** |
| prod compose config | PASS | （個別未） | **PASS** |
| Compose E2E / WAN 実機 / fuzz | 未実行（Linux 前提） | 未実行 | **定義あり・本評価では未実行** |

---

## ブロック別講評（統合）

| ブロック | 採用読み | 合意 |
|:---|:---|:---|
| auth-server | 最強部品のまま | 双方高評価・減点なし |
| assets-server | 前回残件を閉じた。CAS/Grant/Cert 優秀 | 合意 |
| world-server | 門番は白眉。**takeover 権限表が新 P0** | 合意（点数は GPT -4 採用） |
| protocol | AEAD v2 + control plane でさらに伸長 | 合意 |
| shared-sim | 土台は良い。hot-path 確保が未達 | 合意 |
| host-server | worker/tick 成熟。XPBD 接触後退 | 合意 |
| client/app + HostSupervisor | プロセス境界は優秀。**UI host 資格情報寿命** | 合意 |
| launcher | 退役・対象外 | 合意 |
| zenoh/relay/edge | CSR-only closed。軽微のみ | 合意 |
| 横断 | 評価駆動継続。新機能の保証表欠落に注意 | 合意 |

---

## 前回からの変化（解決確認）

| 前回弱点（まとめ） | 今回 |
|:---|:---|
| host Room Token bearer + PoP 欠如 | **closed** — control plane session 署名 + host HTTP `cnf.spk` PoP（x0081） |
| Room 鍵非ローテ / 過去参加者 revoke 不可 | **partial** — epoch/visibility/定期は実装。kick HTTP・friendship 連動は未 → **-2** |
| Scene revision CAS なし | **closed** — expected_revision CAS + 409（x0082） |
| relay-agent `key_pem` | **closed** — CSR-only（x0083） |
| host-token HTTP rate limit | **closed** — 操作別 IP+jti（x0084） |
| GLB 展開後上限なし | **partial→ほぼ closed** — budget あり。画像はデコード後判定が残 → P2 |
| assets mix test フレーク | **closed（原因側）** — hermetic。当日は別因 credo フレーク |
| Allocation throughput / WAN E2E | **closed** — x0088 / x0087 |
| P2 軽微群 | **大半 closed** — dash/desktop は **open 悪化** |

**集計（まとめ 主要行）: closed 7+ / partial 2 / open 1（dash） / superseded 0**  
（Opus まとめ 25 行集計: closed 21 / partial 3 / open 1。GPT: closed 7 / partial 2）

---

## 相違の明示と採用判断

| 項目 | Opus | GPT | 採用 |
|:---|:---|:---|:---|
| 無委譲 takeover → scene 改変 | -3 | -4 | **-4** |
| UI host access JWT 寿命 | -2 | -3 | **-3** |
| Room key kick/friendship | -1 | -2 | **-2** |
| shared-sim hot-path alloc | -1 | -3 | **-2** |
| XPBD body-body 切断 | -2 | — | **-2** |
| handover Presence 先頭 | — | -2 | **-2** |
| scene 構造上限なし | — | -2 | **-2** |
| refresh 平文 prefs | — | -2 | **-1** |
| dash/desktop 肥大 | -2 | （P2 partial） | **-2** |
| 自己改善 / 評価駆動 | +4 帯 | +3 | **+3〜4** |

独立評価として、Opus=定義↔実行の逆転と物理後退、GPT=権限 capability と資格情報寿命・規模保証、を相互補完した。

---

## 同種 OSS 比較（統合）

| 軸 | 比較 | 所見 |
|:---|:---|:---|
| 署名モデル | Matrix server signing | 制御面 + HTTP PoP で Intent 水準に到達 |
| 部屋機密 | MLS / Matrix E2EE | epoch で「入室=永久読取」は解消。Remove/kick 相当が未 |
| 権限モデル | Resonite / VRChat インスタンス | **ワールド保存権とセッション実行権が takeover で混線** |
| CAS | git LFS / IPFS | Grant + Origin Cert v1 は一段上 |
| 物理 | Rapier / avian | SoA 方向は良いが接触・確保で後退 |
| プロセス | Steam 子プロセス | Job Object まで書いた点は優秀。認証 broker が未 |

---

## 次にやるべきこと（トップ5）

詳細は [improvement-plan-2026-11-12.md](improvement-plan-2026-11-12.md)。

1. **HostDelegation + scene 永続書き込みを owner/editor 限定**（takeover host は overlay のみ）+ 否定系
2. **親 client 資格情報 broker**（IPC で access JWT push。refresh は子に渡さない）
3. **XPBD body-body を host で有効化 + hot-path ゼロアロケーション**
4. **kick / friendship → Room key epoch + SceneBudget**
5. **handover 候補 accept 二段階 + dash/desktop 分割**

---

## 評価の前提

- 旧 Elixir 権威実行前提は未使用
- 目標のみ（verifier 統計・billing / market / Formula / OSC 等）は減点せず提案/対象外
- 各評価者は相手文書を読んでいない。まとめは双方詳細を読み、合意・相違・採用を明示した
- assets credo 初回失敗は再実行で消え、フレークとして記録
