# Alchemy — 総合評価レポート（まとめ / 2026-10-30）

- **評価日**: 2026-10-30
- **種別**: 3回目（前回まとめ: [archive/2026-10-28](archive/2026-10-28/evaluation-2026-10-28.md)）
- **評価対象**: `main` @ `d2d9f44`（PR #105 / inventory ladder docs 直後）。前回 `6a0a879` から約 90 コミット（実装 x0038〜x0052、x0053〜x0065 は planned）
- **第1評価者**: [Claude Opus 5.5](opus/opus-evaluation-2026-10-30.md) — **+262 / -36 = +226**
- **第2評価者**: [GPT-5.6 Sol](gpt/gpt-evaluation-2026-10-30.md) — **+86 / -15 = +71**
- **詳細統合**: [強み](specific-strengths-2026-10-30.md) · [弱み](specific-weaknesses-2026-10-30.md) · [提案](specific-proposals-2026-10-30.md) · [改善計画](improvement-plan-2026-10-30.md)

---

## ひとことで言うと

**前回のアプリ層 P0（leave-on-fail / seal oracle / replay / decode limit）はコードと否定系で閉じた。次の穴は、新しく入れた Relay / WAN 門番が「短命・bounded・部屋隔離」に追いついていないことと、CAS access の先着固定である。**

---

## スコアの扱い（合意・相違・採用）

| | プラス | マイナス | 正味 |
|:---|---:|---:|---:|
| 第1（Opus） | +262 | -36 | **+226** |
| 第2（GPT） | +86 | -15 | **+71** |
| **まとめ採用（概算）** | **~+105** | **~-28** | **~+77** |
| 前回まとめ採用 | ~+92 | ~-48 | **~+44** |

### なぜ差が出たか

| 軸 | Opus | GPT | まとめの判断 |
|:---|:---|:---|:---|
| プラスの幅 | 継続項目を細かく再加点（+262） | 検証できた境界に絞る（+86） | 合意コア＋今回閉じた契約。主指標は**マイナス半減と回帰 closed** |
| マイナスの焦点 | wildcard `wire_cap`・平文鍵・relay CI・world RL | 証明書寿命・Allocation bound・CAS access・client E2E | **両方採用** |
| 姿勢 | 「アプリ契約は閉じた。transport が文書と不一致」 | 「公開 WAN 候補へ前進。Relay だけ水準未達」 | **両立** |

採用正味 **約 +77**（前回約 +44）は、「暗号契約の中身が閉じ、スコアが再び跳ねた」という意味。次は relay 契約を閉じればさらに上がる。

---

## ローカル CI（双方一致寄り）

| ターゲット | Opus | GPT | まとめ |
|:---|:---|:---|:---|
| rust（client/host/shared-sim） | PASS | PASS | **PASS** |
| protocol | PASS（39） | PASS（39） | **PASS** |
| launcher | PASS（4） | PASS（4） | **PASS**（前回 FAIL 解消） |
| auth | PASS（122） | PASS | **PASS** |
| world | PASS（65） | PASS（offline audit 再試行） | **PASS**（前回汚染解消） |
| assets | PASS（34） | PASS（offline audit 再試行） | **PASS** |
| prod compose config | PASS | PASS | **PASS** |
| relay-agent | clippy PASS・test 0・ci 外 | 同 | **品質ゲート外（弱点）** |
| Compose E2E / XR 実機 | 未実行 | 未実行 | **定義あり・client trust / WAN 未** |

---

## ブロック別講評（統合）

| ブロック | 採用読み | 合意 |
|:---|:---|:---|
| auth-server | 最強部品のまま | 双方高評価・減点ほぼなし |
| assets-server | Cas/Ref/Grant/予約は優秀。**access 先着は新穴** | GPT の -3 を採用 |
| world-server | canonical seal が白眉。RL 欠如 | Opus の world RL を採用 |
| protocol | decode + gen/rev で大幅伸長 | 合意 |
| host-server | worker 分離・cnf・rate で前回弱点閉鎖 | 合意 |
| client/app | discard・wgpu XR | 合意 |
| launcher | CI 入り。IPC stale 残 | 合意・軽微 |
| zenoh/relay/edge | mTLS 方向は正しい。**寿命・bound・wildcard・平文鍵** | 双方の重大発見を採用 |
| 横断 | 全緑 CI・fuzz・metrics。relay CI / client E2E / README | 合意 |

---

## 前回からの変化（解決確認）

| 前回弱点（まとめ） | 今回 |
|:---|:---|
| 偽 Manifest leave DoS | **closed** — discard + 明示 Leave のみ |
| ManifestSeal 署名 oracle | **closed** — canonical DB scene |
| 署名済み replay | **closed** — generation_gate |
| protobuf decode 上限なし | **closed** — decode_limits |
| prod compose S2S 欠落 | **closed** — compose + smoke |
| world test DB 汚染 | **closed** — test path 隔離 |
| 物理ループ同期 HTTP / 失敗で終了 | **closed** — credential/network worker |
| bind JWT 盗聴（cnf 無し） | **closed** — `cnf.spk` |
| Intent rate / JWKS hard stale | **closed** |
| CAS object/ref・補償 | **closed** |
| fixture / HTTP timeout | **closed** |
| XR vk_loop 二重化 | **closed** — wgpu |
| launcher CI 外 | **closed**（IPC stale は open） |
| metrics 不在 | **closed**（横断 SLO は軽微残） |
| fuzz 不在 | **closed** |
| assets GET 無所有者 | **partial→置換** — 文書化 + grant_required。新規 access 先着へ |
| WAN ACL なし | **partial→置換** — mTLS/role ACL 入。wildcard / 証明書寿命へ |

---

## 相違の明示と採用判断

| 項目 | Opus | GPT | 採用 |
|:---|:---|:---|:---|
| wildcard subscribe → `wire_cap` 漏洩 | -3 | （ACL partial） | **-3**（要否定系確認） |
| 証明書 NotAfter 24h | -1 | -3 | **-3** |
| Allocation JWKS/body/thread bound | 関連 | -3 | **-3** |
| 平文 HTTP で秘密鍵返却 | -2 | — | **-2** |
| CAS access 先着固定 | 非減点 | -3 | **-3** |
| relay CI / test 0 | -2 | -1 | **-2** |
| adversarial が client 未通 | -1 | -2 | **-2** |
| world rate limit | -2 | — | **-2** |
| 自己改善 / 文書規律 | +4 帯 | +3 帯 | **+4** |

独立評価として、Opus=保証表と transport の不一致、GPT=門番契約の寿命・資源・CAS policy、を相互補完した。

---

## 同種 OSS 比較（統合）

| 軸 | 比較 | 所見 |
|:---|:---|:---|
| 署名モデル | Matrix server signing | 「中央が定義から署名」＋「失敗は破棄」に到達。個人規模では稀少 |
| transport 隔離 | NATS / MQTT topic ACL | put 側 role は入った。購読側部屋隔離・短命証は未達 |
| CAS | git LFS / IPFS pin | 物理/参照/Grant は上。access を物理に置いたのは後退気味 |
| 描画 | Bevy OpenXR | wgpu 統合で二重経路負債を解消 |
| 認証 | Keycloak | 範囲は狭いが正しさは同水準 |
| プロセス | 個人 OSS | 評価駆動で 2 日おきに P0 を潰すサイクルは継続 |

---

## 次にやるべきこと（トップ5）

詳細は [improvement-plan-2026-10-30.md](improvement-plan-2026-10-30.md)。

1. **Relay 証明書 NotAfter を `expires_in` と一致** + JWKS hard TTL / body・issuance bound
2. **部屋隔離を保証表どおりにする**（短期: 非保証へ移す / 本対応: L7 or AEAD）+ WAN 否定系
3. **relay 鍵配布を CSR 化し、平文秘密鍵返却をやめる** + Tunnel/TLS 背後
4. **CAS access を Grant/ref 側へ移す**（先着固定の解消）
5. **relay を CI に入れ、headless client trust を adversarial E2E へ**

---

## 評価の前提

- 旧 Elixir 権威実行前提は未使用
- 目標のみ（verifier / billing / market / Formula / OSC / inventory x0054〜x0065 等）は減点せず提案/対象外
- 各評価者は相手文書を読んでいない。まとめは双方詳細を読み、合意・相違・採用を明示した
- Opus の wildcard 指摘は設定読解ベース（zenohd 実機未確認）。実装前に否定系 1 本を推奨
