# Alchemy — 総合評価レポート（まとめ / 2026-10-28）

- **評価日**: 2026-10-28
- **種別**: 2回目（前回まとめ: [archive/2026-10-26](archive/2026-10-26/evaluation-2026-10-26.md)）
- **評価対象**: `main` @ `6a0a879`（PR #64 / x0037 直後）。前回 `683e30f` から約 23 スライス（x001e〜x0037）
- **第1評価者**: [Claude Opus 5.5](opus/opus-evaluation-2026-10-28.md) — **+176 / -36 = +140**
- **第2評価者**: [GPT-5.6 Sol](gpt/gpt-evaluation-2026-10-28.md) — **+69 / -40 = +29**
- **詳細統合**: [強み](specific-strengths-2026-10-28.md) · [弱み](specific-weaknesses-2026-10-28.md) · [提案](specific-proposals-2026-10-28.md) · [改善計画](improvement-plan-2026-10-28.md)

---

## ひとことで言うと

**前回の「境界が開いている」はほぼ閉じた。次の穴は「署名がある＝world が定義を保証した」までの未完と、検証失敗時の振る舞い・prod 継ぎ目である。**  
強い技術デモから、公開 WAN の暗号契約を締め切る段階へ移った。

---

## スコアの扱い（合意・相違・採用）

| | プラス | マイナス | 正味 |
|:---|---:|---:|---:|
| 第1（Opus） | +176 | -36 | **+140** |
| 第2（GPT） | +69 | -40 | **+29** |
| **まとめ採用（概算）** | **~+92** | **~-48** | **~+44** |
| 前回まとめ採用 | ~+75 | ~-72 | **~+3** |

### なぜ差が出たか

| 軸 | Opus | GPT | まとめの判断 |
|:---|:---|:---|:---|
| プラスの幅 | 新機能を細かく加点（+80 増の一部は分割効果） | 検証できた境界に絞る | 合意コア + 自己改善/文書規律。比較指標は**マイナス半減**を主に見る |
| マイナスの焦点 | leave-on-fail・prod compose・test DB | seal oracle・replay・decode limit・assets GET | **両方採用**（重なりは折衷、片方発見は採用） |
| 姿勢 | 「致命傷は塞いだ。継ぎ目の穴が残る」 | 「デモは強い。公開 WAN には未達」 | **両立** — 前回比は大幅改善、公開前提の暗号契約は未完 |

採用正味 **約 +44**（前回約 +3）は、「境界の仕組みが揃い、スコアが急上昇した」という意味。楽観（+140）でも厳格（+29）でもなく、**oracle / replay / leave-on-fail / decode limit を閉じればさらに跳ねる**と読む。

---

## ローカル CI（双方一致寄り）

| ターゲット | Opus | GPT | まとめ |
|:---|:---|:---|:---|
| rust（client/host/shared-sim） | PASS | PASS | **PASS** |
| protocol | PASS（20） | PASS（20） | **PASS** |
| auth | PASS（124） | PASS | **PASS** |
| assets | PASS（13） | PASS | **PASS**（前回 FAIL 解消） |
| alchemy_auth_client | PASS（9） | — | **PASS** |
| world | 1 FAIL（DB 汚染）/ 隔離で全通 | 1 FAIL（stale test DB） | **ローカル赤・隔離で緑** |
| launcher | clippy FAIL・test 0・CI 外 | IPC stale 指摘 | **品質ゲート外** |
| Compose E2E | 未実行（定義確認） | 定義確認 | **定義あり・adversarial 未** |

---

## ブロック別講評（統合）

| ブロック | 採用読み | 合意 |
|:---|:---|:---|
| auth-server | 最強部品のまま。refresh / social / ClientIp まで伸びた | 双方高評価 |
| assets-server | Quota/CI 隔離は良い。GET 認可・object/ref は未 | GPT の -4 を折衷 -3 で継続 |
| world-server | owner/visibility/attest は優秀。**seal が oracle** | GPT 重大発見を採用 |
| protocol | PIN/TRS/wire_crypto/delivery で大幅伸長。decode limit 欠如 | 合意（弱点は GPT） |
| host-server | 入力ゲート・Presence・refresh は良い。物理ループ同期 HTTP | 合意 |
| client/app | Frame 検証は閉じた。**leave-on-fail / replay** | 双方の重大穴を採用 |
| launcher | 実装化。CI 外・IPC stale | 合意・軽微 |
| zenoh/edge | Tunnel/TLS は直った。ACL 未・**prod compose 起動不能** | Opus の prod -3 採用 |
| 横断 | E2E/衛生は前進。world test DB・metrics・adversarial 薄い | 合意 |

---

## 前回からの変化（解決確認）

双方がコード再読で「解決」とした主な項目（まとめでも採用）:

| 前回弱点 | 今回 |
|:---|:---|
| Tunnel dev 公開 / seed / host_control | prod compose・`/dev/*` 404・seed fail-closed・IPC |
| 任意 host role / Room Token 非更新 / JWKS 同期 | owner のみ・80% refresh・SWR |
| liveliness なりすまし / Intent JWT なりすまし | jti 束縛・Ed25519+seq（bind 盗聴は残） |
| 偽 Frame / delta 非収束 / 巨大 move | 署名・tick・snapshot・clamp |
| WAN 平文 / assets CI 汚染 / Quota race | TLS / 三重隔離 / 条件 UPDATE |
| 定義 vs 実行逆転 / PIN 非ワイヤ / TRS 欠如 | Manifest 駆動・PIN・TRS（一部残） |
| E2E 不在 / advisory 非 gate / unsafe 無コメント | Compose E2E・hex.audit/cargo-deny・deny lint |

---

## 相違の明示と採用判断

| 項目 | Opus | GPT | 採用 |
|:---|:---|:---|:---|
| ManifestSeal 署名 oracle | （未個別） | -4 | **-4** |
| Manifest/Presence replay | （未個別） | -4 | **-4** |
| 偽 Manifest leave DoS | -3 | （replay 近傍） | **-3** |
| protobuf decode 上限なし | （未個別） | -4 | **-3** |
| prod compose 起動不能 | -3 | （未個別） | **-3** |
| assets GET 無所有者検査 | 弱 | -4 | **-3** |
| 物理ループ同期 HTTP | -2 | -2 | **-2** |
| bind JWT 盗聴 | -2 | （未個別） | **-2** |
| WAN ACL/mTLS | -1 | -3 | **-2** |
| world test DB 汚染 | -2 | -2 | **-2** |
| 自己改善サイクル / 文書規律 | +4 帯 | +3 帯 | **+4** |

独立評価として、**前回と同様に相手が見落とした重大穴を相互補完した**のが価値。今回は Opus=運用継ぎ目・失敗時振る舞い、GPT=暗号契約の中身の保証。

---

## 同種 OSS 比較（統合）

| 軸 | 比較 | 所見 |
|:---|:---|:---|
| 認証 | Keycloak / gen.auth | refresh family・social SSoT は個人規模で突出 |
| 空間同期 | Resonite / VRChat | Manifest×姿勢×host 署名は同筋。canonical seal・anti-replay・補間は商用必須級が未 |
| 署名モデル | Matrix server signing | 「中央が認めた鍵」までは近い。内容ハッシュの正本照合が欠けると oracle |
| Zenoh | 公式デプロイ | TLS は入った。ACL/QoS/mTLS はまだ半分 |
| エンジン | Bevy + OpenXR | XR 生 Vulkan 二重経路は継続負債 |
| プロセス | 個人 OSS | 2 日で 23 スライスの評価駆動改善は稀少 |

---

## 次にやるべきこと（トップ5）

詳細は [improvement-plan-2026-10-28.md](improvement-plan-2026-10-28.md)。

1. **検証失敗は破棄**（leave-on-fail 廃止）+ 否定系 E2E
2. **ManifestSeal を world DB scene 由来の canonical に限定**（oracle 廃止）
3. **generation / revision 単調ゲート**（replay 拒否）
4. **prod compose に S2S env を通し起動 smoke**、world テスト DB 隔離
5. **decode byte/count limit** + assets GET 方針（文書 or BlobGrant）

---

## 評価の前提

- 旧 Elixir 権威実行前提は未使用
- 目標のみ（verifier / billing / market / Formula / OSC 等）は減点せず提案/対象外
- 各評価者は相手文書を読んでいない。まとめは双方詳細を読み、合意・相違・採用を明示した
