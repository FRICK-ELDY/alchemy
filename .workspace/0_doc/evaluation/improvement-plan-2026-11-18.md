# Alchemy — 改善提案書（まとめ / 2026-11-18）

> 根拠: [specific-weaknesses-2026-11-18.md](specific-weaknesses-2026-11-18.md)  
> 前回計画: [archive/2026-11-12/improvement-plan-2026-11-12.md](archive/2026-11-12/improvement-plan-2026-11-12.md)  
> 品質本線（物理）: [mvcc_xpbd.md](../../roadmap/mvcc_xpbd.md)  
> 規模本線（配信）: [spatial_interest.md](../../roadmap/spatial_interest.md)  
> 目的: 採用マイナス ~-18 を潰し、正味 ~+132 をさらに押し上げる。

---

## 方針

1. **social revoke を「書いたら閉じた」にしない** — 発火条件・推移走査・outbox・kick 拒否を一続きで閉じる
2. **既定で走る物理経路を、証明した経路と揃える** — LBVH O(n) + Epoch clone 削減。MVCC page-alloc 閉鎖を host 全体 zero-alloc と読まない
3. **重い入口に流量制御を足す** — assets RL と `scene/save` 抜け
4. **既定にした cell 本線に回復と E2E を追随させる** — ACL と検査項目を突き合わせる
5. **衛生は小さく早く** — overlay revision・CI volume・trust-boundary・smoke・分割

前回 P0（HostDelegation / AUTH_PUSH / handover / SceneBudget / vault）と P1/P2 大半はコードで閉じた。残課題と新規をこの版に載せ替える。

---

## P0 — 今すぐ（revoke・物理計算量・流量）

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P0-1 | social revoke 完了 | -3 | 全 unfollow で `relation_changed`。world は影響 owner（`friends_plus`/`followers_plus` 含む）を再評価して epoch CAS。auth に durable outbox + 冪等 consume | 通常 unfollow 後、旧鍵で新 epoch 平文が読めない否定系。world 一時停止後も outbox 再送で閉じる |
| P0-2 | LBVH O(n) + サブステップ refit | -2 | `(code, leaf_index)` で AABB を添字引き。サブステップ 2 回目以降は refit。n=1024/4096 の時間計測（`#[ignore]` 可） | **完了（x00d8）** — 構築 O(n)。SceneBudget 上限で 1 tick 時間予算を文書化 |
| P0-3 | assets HTTP rate limit | -2 | path 依存 `shared/alchemy_rate_limit` を assets に導入。`blob_put`/`vcs_push`/`vcs_negotiate`/`spawn_ticket` を IP×sub。push は同時 1 本セマフォ | 連打で 429。Port プール飢餓の否定系 |

---

## P1 — 既定経路の保証・配信・kick

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P1-1 | 既定 Epoch 経路の clone/確保削減 | -2 | `clone_from`/swap・scratch 再利用。カウンタアロケータを Epoch 経路にも。MVCC default 切替はベンチ条件付き | tick あたり確保回数の上限テスト。p95 予算を文書化 |
| P1-2 | kick 拒否リスト | -1 | Room に `denied_subs`。Token 発行・takeover・handover accept で拒否。deny+rotate を 1 CAS。client 導線 | public 部屋で kick 後に新 Token が 403 |
| P1-3 | cell 配信の回復 | -1 | 占有セルの低頻度キーフレーム、またはセル連番→欠番で snapshot。client は Moore 外の古 entity を prune | soft-put 失敗・remove 欠落の回帰 |
| P1-4 | 新ワイヤ経路の E2E | -1 | e2e-probe が Moore cell 購読。否定系: guest cell put 不達、guest `sim_state` 購読拒否、SpawnTicket 二重使用 403 | WAN E2E が既定本線を検査。ACL キー一覧と検査項目の突合 |

---

## P2 — 門番抜け・衛生

| ID | 項目 | 根拠 | 作業イメージ |
|:---|:---|:---|:---|
| P2-1 | `scene/save` を rate limit 対象へ | -1 | `:host_scene` または `:host_save`。連打 429 の否定系 |
| P2-2 | overlay revision を canonical 更新時に 0 へ | -1 | save/scene CAS で overlay リセット。packed 単調は維持。上限境界テスト |
| P2-3 | assets CI volume の状態依存を潰す | -1 | 匿名 volume、または `-Werror` 前に `mix clean` / test-support 清掃 |
| P2-4 | dash / sim / network_worker 分割 | -2 帯 | タブ単位・純関数の cell 差分計画・`publish/{room,cell,lod}` |
| P2-5 | `smoke-rapier-host.sh` 削除または現行バナーへ | -2 帯 | README を `make ci-e2e` 等に置換 |
| P2-6 | trust-boundary に edit / sim_state / SpawnTicket / VCS 行 | -2 帯 | スライス完了条件に「trust-boundary 該当行」を入れる |

---

## P3 — アーキテクチャ（スライス推奨）

| ID | 項目 | 根拠 | 備考 |
|:---|:---|:---|:---|
| P3-1 | verifier Spot-Check orchestration | 目標 | Noop 配線のみ OSS。統計は Proprietary |
| P3-2 | Spatial VCS clone/fetch・Cert 鍵履歴 | 提案 | Origin Cert v2 の次段 |
| P3-3 | S2S 連合 / World 一覧交換 | 目標 | |
| P3-4 | billing / market / Relay QoS | 目標 | ディレクトリ未設置 |
| P3-5 | Dual-AST / Wake / OSC / LOD ビルド / audio | 目標 | 減点しない |
| P3-6 | 署名付き installer / SBOM / 自動更新 | 提案 | OS vault 後の配布 trust |
| P3-7 | Spatial Interest 深化（AOI snapshot・規模 gate） | 規模本線 | 正本 [`spatial_interest.md`](../../roadmap/spatial_interest.md) |
| P3-8 | MVCC default 切替・複数 page | 品質本線 | 正本 [`mvcc_xpbd.md`](../../roadmap/mvcc_xpbd.md)。ベンチ条件を満たしてから |

---

## 前回計画のクローズ状況（要約）

| 前回帯 | 状態 |
|:---|:---|
| P0-1 HostDelegation | **完了**（x009d / x009e） |
| P0-2 資格情報 broker | **完了**（x009f） |
| P1-1 kick/friendship→epoch | **部分完了** — rotate+通知は入った。発火・推移・outbox・拒否リストは今回 P0-1 / P1-2 |
| P1-2 host XPBD 接触 | **完了**（x00a4 / x00ad） |
| P1-3 hot-path zero-alloc | **部分完了** — MVCC page-alloc closed。既定 Epoch は今回 P1-1。LBVH は P0-2 |
| P1-4 handover accept | **完了**（x00a1） |
| P1-5 SceneBudget | **完了**（x00a2） |
| P1-6 refresh OS vault | **完了**（x00cf） |
| P2-1〜P2-8 | **大半完了**。dash 分割・CI volume・docs smoke に残件 → 今回 P2 |

---

## 推奨スライス順

1. **social revoke 完了**（P0-1）— 偽の closed を先に潰す
2. **LBVH O(n)+refit**（P0-2）— 修正量小・効果大
3. **assets RL + scene/save RL**（P0-3 / P2-1）
4. **kick deny + cell 回復 + E2E**（P1-2 / P1-3 / P1-4）
5. **Epoch clone 削減**（P1-1）— MVCC default はベンチ後（P3-8）
6. **P2 衛生**（overlay / CI / trust-boundary / smoke / 分割）— 並走可
7. **P3** — Interest 深化・VCS clone・verifier・配布。品質本線と規模本線を混同しない

スライス完了条件に必ず含める:

- **新しい保証は否定系（単体＋可能な範囲で E2E）で確認**
- **保証表・trust-boundary・非保証欄とコードが一致**
- **「closed」は既定で走る経路について言う**（証明専用 backend だけの閉鎖にしない）
- **規模 CCU 宣言は [`spatial_interest.md`](../../roadmap/spatial_interest.md) のゲートに従う**
