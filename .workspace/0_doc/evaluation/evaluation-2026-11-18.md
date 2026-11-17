# Alchemy — 総合評価レポート（まとめ / 2026-11-18）

- **評価日**: 2026-11-18
- **種別**: 6回目（前回まとめ: [archive/2026-11-12](archive/2026-11-12/evaluation-2026-11-12.md)）
- **評価対象**: `main` @ `57ca452`（PR #247 / x00d6 直後）。前回 `8833821` から **135 コミット**（x009d〜x00d6、HostDelegation、credential broker、MVCC×XPBD、Spatial Interest、Spatial VCS、P2 衛生）
- **第1評価者**: [Claude Opus 5.5](opus/opus-evaluation-2026-11-18.md) — **+467 / -15 = +452**
- **第2評価者**: [GPT-5.6 Sol](gpt/gpt-evaluation-2026-11-18.md) — **+89 / -6 = +83**
- **詳細統合**: [強み](specific-strengths-2026-11-18.md) · [弱み](specific-weaknesses-2026-11-18.md) · [提案](specific-proposals-2026-11-18.md) · [改善計画](improvement-plan-2026-11-18.md)

---

## ひとことで言うと

**前回の P0（takeover 過大権限・UI host 資格情報）は権限モデルごと閉じた。次の穴は、social revoke の深さ、既定物理の O(n²)、assets の流量制御、そして既定にした cell 本線の回復と E2E 追随である。**

---

## スコアの扱い（合意・相違・採用）

| | プラス | マイナス | 正味 |
|:---|---:|---:|---:|
| 第1（Opus） | +467 | -15 | **+452** |
| 第2（GPT） | +89 | -6 | **+83** |
| **まとめ採用（概算）** | **~+150** | **~-18** | **~+132** |
| 前回まとめ採用 | ~+132 | ~-23 | **~+109** |

### なぜ差が出たか

| 軸 | Opus | GPT | まとめの判断 |
|:---|:---|:---|:---|
| プラスの幅 | 継続項目を細かく再加点（+467） | 検証できた境界に絞る（+89） | 合意コア＋今回閉じた契約。主指標は**旧 P0 閉鎖と新穴** |
| マイナスの焦点 | LBVH O(n²)・assets RL・cell/E2E・衛生 | social revoke -3・hot-path clone・CI flake | **両方採用（厳しめ側）** |
| 姿勢 | 「門番は厚くなった。新既定経路の足元が残る」 | 「closed と書いた revoke は部分。zero-alloc を過大解釈するな」 | **両立** |

採用正味 **約 +132**（前回約 +109）は、「旧 P0 を閉じた伸び」から「social revoke / LBVH / assets RL / 既定経路 clone」の減点を差し引いた結果。

---

## ローカル CI（双方＋親エージェント追試）

| ターゲット | Opus | GPT | まとめ |
|:---|:---|:---|:---|
| client workspace | PASS（708） | PASS | **PASS** |
| protocol | PASS（124） | PASS | **PASS** |
| relay-agent | PASS（32） | PASS | **PASS** |
| auth | PASS（134） | PASS | **PASS** |
| world | PASS（232） | PASS | **PASS** |
| assets | 初回 FAIL → 再実行/volume 掃除後 PASS（240） | 同様 flake 記録 | **PASS（volume 依存を弱点 -1）** |
| shared/alchemy_* | PASS（25） | PASS | **PASS** |
| prod compose config | PASS | PASS | **PASS** |
| Compose E2E / WAN 実機 / fuzz | 未実行（新経路未配線も確認） | 未実行 | **定義あり・本評価では未実行** |

並行 Docker / Hex timeout / stale `_build` で初回 `ci.sh all` が落ちる経路は再現した。順次実行＋assets volume 掃除後は Elixir 3 本も緑。

---

## ブロック別講評（統合）

| ブロック | 採用読み | 合意 |
|:---|:---|:---|
| auth-server | 最強部品のまま。revoke 通知は導入済みだが深さ不足 | 合意（減点は social 側へ） |
| assets-server | VCS / SpawnTicket 優秀。**RL ゼロが新 P1** | 合意（Opus -2 採用） |
| world-server | HostDelegation は白眉。revoke/kick/save RL/overlay が周辺残 | 合意 |
| protocol | PIN 0.24・edit/sim_state/vcs で伸長 | 合意 |
| shared-sim | 部品は良い。**既定経路が O(n²)+clone** | 合意 |
| host-server | edit / Interest 成熟。一枚岩再成長 | 合意 |
| client/app + HostSupervisor | broker/vault/KeyStale 優秀。cell 回復不足 | 合意 |
| launcher | 退役・対象外 | 合意 |
| zenoh/relay/edge | CSR-only / ureq / ACL 明示で健全 | 合意 |
| 横断 | 評価駆動継続。E2E・trust-boundary 追随が遅れた | 合意 |

---

## 前回からの変化（解決確認）

| 前回弱点（まとめ） | 今回 |
|:---|:---|
| 可視参加者の無委譲 takeover → canonical 改変 | **closed** — HostDelegation + overlay + 否定系（x009d/x009e） |
| UI host が ~15 分で資格情報更新不能 | **closed** — AUTH_PUSH broker（x009f） |
| kick / friendship 即時 revoke 未完 | **partial** — rotate + relation_changed はある。拒否リスト・unfollow 発火・推移走査・outbox が残 → **-3/-1** |
| XPBD body-body 接触切断 | **closed** — 不等式接触 + host bounds（x00a4/x00ad） |
| shared-sim / host hot-path alloc | **open/partial** — MVCC page-alloc closed。既定 Epoch clone 継続 + LBVH O(n²) 新規 |
| handover が Presence 先頭を指名 | **closed** — accept 二段階（x00a1） |
| canonical scene 構造上限なし | **closed** — SceneBudget（x00a2） |
| refresh 平文 prefs | **closed** — OS vault（x00cf） |
| dash/desktop 肥大 | **partial** — desktop 分割済、dash 残 |
| world 軽微群 / GLB / KeyStale / relay curl 等 | **大半 closed** — scene/save RL・smoke・CI volume・trust-boundary が残/新規 |

**集計（まとめ 主要行）: closed 7+ / partial 3 / open 1（hot-path 既定） / superseded 0**  
（Opus: closed 7 / partial 3 / open 1。GPT: closed 6 / partial 2）

---

## 相違の明示と採用判断

| 項目 | Opus | GPT | 採用 |
|:---|:---|:---|:---|
| social revoke（unfollow/推移/outbox） | 通知を kick 帯に圧縮 | -3 | **-3** |
| kick 拒否リスト | -1 | （social に包含寄り） | **-1**（別件として残す） |
| LBVH O(n²) | -2 | — | **-2** |
| assets HTTP RL 無し | -2 | — | **-2** |
| 既定経路 sheet clone | -1 | -2 | **-2** |
| cell 回復 / E2E 追随 | 各 -1 | — | **各 -1** |
| scene/save RL・overlay 上限 | 各 -1 | — | **各 -1** |
| assets CI volume | -1 | -1 | **-1** |
| dash/host 一枚岩/smoke/trust-boundary | 群 -1〜-2 | — | **まとめて -2** |
| 自己改善 / 評価駆動 | +4 帯 | +4 | **+4** |

独立評価として、Opus=新既定経路の計算量と検証網、GPT=revoke の過大「closed」判定と hot-path 過大解釈の是正、を相互補完した。

---

## 同種 OSS 比較（統合）

| 軸 | 比較 | 所見 |
|:---|:---|:---|
| 権限モデル | Resonite / VRChat | 実行権と保存権の分離は Resonite 線に並んだ。ban 相当の拒否リストは未 |
| 部屋機密 | MLS / Matrix E2EE | epoch 前進は Remove に近い。followers/推移での取りこぼしと public 再入室が残る |
| CAS / VCS | git LFS / receive-pack | Grant + 決定的 VCS + Origin Cert v2 は一段上。clone/鍵履歴は未 |
| 物理 | Rapier / avian | 接触復帰は良い。既定 BVH は毎サブステップ O(n²) で劣る |
| 配信 | Unreal Replication Graph | cell/lod 先行。キーフレーム/回復が未 |
| 資格情報 | OAuth BCP | 親 broker + OS vault は BCP どおり |
| 流量制御 | Phoenix+Hammer 一般 | auth/world は上回る。assets だけ無い |

---

## 次にやるべきこと（トップ5）

詳細は [improvement-plan-2026-11-18.md](improvement-plan-2026-11-18.md)。

1. **social revoke を完了する**（全 unfollow 発火・推移 owner 再評価・durable outbox）+ kick 拒否リスト
2. **LBVH を O(n) + サブステップ refit**し、既定経路の clone/確保を測って縛る
3. **assets に `alchemy_rate_limit`**（push 同時 1 本）+ `scene/save` RL
4. **cell 本線の回復（キーフレーム/連番）+ WAN E2E を新既定キーへ追随**
5. **衛生**: overlay revision リセット、CI volume、trust-boundary、smoke 削除、dash/host 分割

---

## 評価の前提

- 旧 Elixir 権威実行前提は未使用
- 目標のみ（verifier 統計・billing/market・Dual-AST・OSC 等）は減点せず提案/対象外
- Proprietary（pgvector 閾値等）を OSS に求めていない
- 各評価者は相手文書を読んでいない。まとめは双方詳細を読み、合意・相違・採用を明示した
- improvement-plan の「closed」注記は信じず、双方がコード再読で回帰判定した
