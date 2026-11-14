# Alchemy — 総合評価レポート（まとめ / 2026-11-05）

- **評価日**: 2026-11-05
- **種別**: 4回目（前回まとめ: [archive/2026-10-30](archive/2026-10-30/evaluation-2026-10-30.md)）
- **評価対象**: `main` @ `c491016`（PR #142 / x007b scene despawn 直後）。前回 `d2d9f44` から **114 コミット**（x0066〜x007b、inventory ladder 実装、x0053 hot switch、4階層ライセンス）
- **第1評価者**: [Claude Opus 5.5](opus/opus-evaluation-2026-11-05.md) — **+331 / -23 = +308**
- **第2評価者**: [GPT-5.6 Sol](gpt/gpt-evaluation-2026-11-05.md) — **+71 / -16 = +55**
- **詳細統合**: [強み](specific-strengths-2026-11-05.md) · [弱み](specific-weaknesses-2026-11-05.md) · [提案](specific-proposals-2026-11-05.md) · [改善計画](improvement-plan-2026-11-05.md)

---

## ひとことで言うと

**前回の Relay / WAN / CAS P0 は 1 週間でコードと否定系で閉じた。次の穴は、新しく正本になった Room 鍵の寿命・host Token の運び方・canonical scene の revision CAS である。**

---

## スコアの扱い（合意・相違・採用）

| | プラス | マイナス | 正味 |
|:---|---:|---:|---:|
| 第1（Opus） | +331 | -23 | **+308** |
| 第2（GPT） | +71 | -16 | **+55** |
| **まとめ採用（概算）** | **~+118** | **~-24** | **~+94** |
| 前回まとめ採用 | ~+105 | ~-28 | **~+77** |

### なぜ差が出たか

| 軸 | Opus | GPT | まとめの判断 |
|:---|:---|:---|:---|
| プラスの幅 | 継続項目を細かく再加点（+331） | 検証できた境界に絞る（+71） | 合意コア＋今回閉じた契約。主指標は**旧 P0 閉鎖と新穴の採用** |
| マイナスの焦点 | host Token bearer/PoP・鍵非ローテ・Port 直列・assets flake | 鍵 revoke・scene CAS・key_pem・GLB budget・host RL | **両方採用** |
| 姿勢 | 「transport は閉じた。inventory 制御面と鍵寿命が未達」 | 「AEAD は効くが寿命・原子性・資源上限が次」 | **両立** |

採用正味 **約 +94**（前回約 +77）は、「Relay/CAS を閉じた伸び」から「新 P0 の減点」を差し引いた結果。次は Room 鍵 epoch + PoP + scene CAS を閉じればさらに上がる。

---

## ローカル CI（双方一致寄り）

| ターゲット | Opus | GPT | まとめ |
|:---|:---|:---|:---|
| rust（client workspace） | PASS（275）※初回 XR flake あり→再実行 PASS | 同 | **PASS**（flake 記録） |
| protocol | PASS（67） | PASS（67） | **PASS** |
| launcher | PASS（8） | PASS（8） | **PASS** |
| relay-agent | PASS（16） | PASS（16） | **PASS**（前回 test 0 / CI 外 → **closed**） |
| auth | PASS（130） | PASS（130） | **PASS** |
| world | PASS（116） | PASS（116） | **PASS** |
| assets | 初回 FAIL 54 → 再実行 PASS 168 | 初回 FAIL 29 → 再実行 PASS | **PASS（フレーク）** |
| prod compose config | PASS | （個別確認） | **PASS** |
| Compose E2E / WAN 実機 | 未実行（静的確認） | 未実行 | **定義あり・実 host 一往復は未** |

---

## ブロック別講評（統合）

| ブロック | 採用読み | 合意 |
|:---|:---|:---|
| auth-server | 最強部品のまま | 双方高評価・減点なし |
| assets-server | access→Grant 再設計は優秀。mime/Port/flake 残 | 合意 |
| world-server | seal/CSR は白眉。**鍵寿命・PoP・scene CAS・host RL** | 双方の重大発見を採用 |
| protocol | AEAD 追加でさらに伸長 | 合意 |
| host-server | worker/session 成熟。制御面 Token と scene 原子性が露出 | 合意 |
| client/app | trust 共有・hot switch。GLB budget / take-out UI | 合意 |
| launcher | discovery で前回弱点閉鎖 | 合意 |
| zenoh/relay/edge | 旧 P0 閉鎖。互換 `key_pem` と鍵 revoke が残 | 合意 |
| 横断 | 全緑相当 CI・metrics。assets flake / smoke / shared | 合意 |

---

## 前回からの変化（解決確認）

| 前回弱点（まとめ） | 今回 |
|:---|:---|
| WAN ACL wildcard で `wire_cap` 部屋隔離不成立 | **closed→置換** — Room AEAD + 保証表修正 + WAN 否定系。過去参加者 revoke は新規 P0 へ |
| 証明書 NotAfter 24h | **closed** — `-not_after` ≈300s |
| Allocation JWKS/body/thread/jti | **closed** |
| relay 秘密鍵平文返却 | **partial** — world CSR 正経路 closed。Agent `key_pem` 残 → **-2** |
| CAS access 先着固定 | **closed** — カラム削除 + Grant/PlaceRead |
| relay-agent CI/test 外 | **closed** — 16 tests + rust-ci + ci.sh |
| adversarial E2E が client trust 外 | **closed** — `alchemy-client-trust` + trust-probe |
| world rate limit / Room 上限 | **partial→置換** — auth-JWT scope closed。host-token scope は新規 |
| 相関 ID / metrics | **closed** — x007a |
| launcher IPC stale | **closed** — discovery |
| README / client.md / vision 鮮度 | **closed** — README 追従。client / wan / world / vision は **x008a**（P2-10） |
| 軽微群 | **partial** — x0074〜x0079 大半 closed。残りは P2 |

**集計（まとめ 12 行）: closed 8 / partial 3 / open 0 / superseded→置換 1（wildcard→鍵 revoke）**

---

## 相違の明示と採用判断

| 項目 | Opus | GPT | 採用 |
|:---|:---|:---|:---|
| host Token bearer + PoP 欠如 → 乗っ取り連鎖 | -3 | — | **-3** |
| Room 鍵非ローテ / 過去参加者 revoke 不可 | -2 | -3 | **-3** |
| Scene revision CAS なし | — | -3 | **-3** |
| relay-agent `key_pem` | -1 partial | -2 open | **-2** |
| host-token HTTP rate limit | -1 | -2 | **-2** |
| GLB 展開後上限なし | — | -2 | **-2** |
| assets mix 並列フレーク | -1 | 記録のみ | **-1** |
| 自己改善 / 評価駆動 | +4 帯 | +3〜4 帯 | **+4** |

独立評価として、Opus=制御面の Token 運び方と PoP、GPT=鍵寿命・scene 原子性・資源上限、を相互補完した。

---

## 同種 OSS 比較（統合）

| 軸 | 比較 | 所見 |
|:---|:---|:---|
| 署名モデル | Matrix server signing | 中央定義→署名＋失敗は破棄は継続。inventory 制御面だけ Intent 水準に未到達 |
| 部屋機密 | MLS / Matrix E2EE | AEAD 導入は大きな前進。epoch/revoke が無いと「入室＝永久読取」 |
| CAS | git LFS / IPFS | access 除去後は権利モデルが一段上 |
| transport | NATS / MQTT ACL | put role + 短命証は達した。購読は wildcard のまま AEAD で補完 |
| 描画 | Bevy OpenXR | hot switch + wgpu。展開後 budget は未 |
| プロセス | 個人 OSS | 2 週間で旧 P0 を潰す評価駆動は継続 |

---

## 次にやるべきこと（トップ5）

詳細は [improvement-plan-2026-11-05.md](improvement-plan-2026-11-05.md)。

1. **制御面を session 署名へ + world host API に PoP**（Token を Zenoh に載せない）
2. **Room 鍵 epoch / visibility・kick でローテ** + 保証表に寿命を書く
3. **Scene append/delete の revision CAS** + 競合テスト
4. **relay-agent を CSR-only 化または退役**
5. **host-token RL + GLB decode budget + assets test hermetic**

---

## 評価の前提

- 旧 Elixir 権威実行前提は未使用
- 目標のみ（verifier 統計・billing / market / Dual-AST Kernel / OSC 等）は減点せず提案/対象外
- 各評価者は相手文書を読んでいない。まとめは双方詳細を読み、合意・相違・採用を明示した
- assets / client の初回失敗は再実行で消え、恒常バグではなくフレークとして記録
