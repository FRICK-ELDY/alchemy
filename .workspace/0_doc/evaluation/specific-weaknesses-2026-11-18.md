# Alchemy — マイナス点統合一覧（まとめ / 2026-11-18）

> 根拠: [Opus](opus/opus-specific-weaknesses-2026-11-18.md) · [GPT](gpt/gpt-specific-weaknesses-2026-11-18.md)  
> 採用ルール: 双方合意は折衷または厳しめ側、片方発見の実害は採用。前回まとめ（[archive/2026-11-12](archive/2026-11-12/specific-weaknesses-2026-11-12.md)）で **closed** と確認された項目は載せない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 軽微な改善余地 |
| -2 | 重要な機能・設計の欠如 |
| -3 | バグ・クラッシュ・性能劣化を起こしうる明確な欠陥 |
| -4 | 価値命題を損なう重大な欠如 |
| -5 | 根幹を揺るがす致命的欠陥 |

| 採用合計（概算） | **-18** |

---

## P0/P1 — 新本線の足元（revoke・物理・流量・配信）

### auth / world — social revoke

- **followers 系・推移関係の Room 鍵 revoke が不完全で、通知も永続化されない** `-3`（GPT -3 / Opus は kick・通知を -1 帯）
  > `unfollow` は accepted friendship があるときしか `relation_changed` を送らず、通常 follower の離脱では `followers` Room の鍵が回らない。world は通知二者が owner の Room だけを走査するため、第三者 owner の `friends_plus` / `followers_plus` から外れた viewer の鍵も回らない。通知は fire-and-forget（retry/outbox 無し）。該当主体は次の定期回転（既定 24h）まで旧 epoch を復号できる。
  > 採用: **-3**（GPT。Opus の「通知に再試行なし」を包含し、発火条件・推移走査まで含む）
  > 対象ファイル: `auth-server/lib/auth/social.ex`, `auth-server/lib/auth/world_notify.ex`, `world-server/lib/world/rooms/room_key.ex`

- **kick は鍵ローテだけで拒否リストが無く、public 部屋では再入室で新鍵を取れる** `-1`（Opus -1）
  > owner の `room_key/rotate` と friendship 連動は入ったが、`denied_subs` 等が無く、public / link では kick が短時間切断にしかならない。client に kick 導線も無い。
  > 採用: **-1**（social revoke とは別の「再入室拒否」欠如）
  > 対象ファイル: `world-server/lib/world/rooms/room_key.ex`, `world-server/lib/world_web/router.ex`

### shared-sim / host — 既定物理経路

- **既定物理経路の LBVH 構築が O(n²) で、毎サブステップ（60Hz × 4）作り直される** `-2`（Opus -2）
  > morton の radix sort 後、葉 AABB を `find` で線形探索。`EpochXpbd` 既定かつ host は常に `scene_bounds` 付き。SceneBudget 4096 と host 60Hz が層間で不整合。
  > 採用: **-2**
  > 対象ファイル: `client/shared-sim/src/spatial.rs`, `client/shared-sim/src/xpbd.rs`, `client/host-server/src/sim.rs`

- **MVCC page 経路を追加しても、既定 host は全 EpochSheet clone を継続する** `-2`（GPT -2 / Opus -1）
  > 既定 `EpochXpbd` は tick あたり sheet clone 3 回 + 一時 `Vec` / `HashSet`。page-alloc ≤2 は MVCC 経路のみ。本番経路と証明経路が別。
  > 採用: **-2**（GPT 厳しめ）
  > 対象ファイル: `client/host-server/src/sim.rs`, `client/host-server/src/sim_backend.rs`, `client/shared-sim/src/xpbd.rs`

### assets — 流量

- **assets-server に HTTP レート制限が無く、重い VCS push / negotiate / SpawnTicket が素通し** `-2`（Opus -2）
  > コア 3 サービスで唯一 RL 無し。push は最大 12 MiB・固定 Port プール。Quota は容量のみで CPU/Port 占有を抑えない。
  > 採用: **-2**
  > 対象ファイル: `assets-server/mix.exs`, `assets-server/lib/assets_web/router.ex`, `assets-server/lib/assets/vcs_object/worker.ex`

### client — Spatial Interest 本線

- **既定にした cell 配信に、取りこぼしからの回復手段が無い** `-1`（Opus -1）
  > soft-put 失敗で姿勢が古いまま残り、remove 欠落で遠方ゴーストが残る。旧 Room frame の gap→snapshot 網が cell 既定では粗い。
  > 採用: **-1**
  > 対象ファイル: `client/app/src/bus/frame.rs`, `client/host-server/src/network_worker.rs`

- **新しいワイヤ経路（cell / lod / edit / sim_state）と SpawnTicket が E2E に無い** `-1`（Opus -1）
  > WAN ACL は新キーを足したが、WAN E2E は既定 client が購読しない Room frame を検査。ACL 誤記でも CI 緑のままになりうる。
  > 採用: **-1**
  > 対象ファイル: `.scripts/smoke-compose-wan-e2e.sh`, `client/e2e-probe/src/main.rs`, `relay-server/config.wan.e2e.json5`

---

## P2 — 門番抜け・衛生・CI

- **`POST …/scene/save` だけ rate limit 対象外** `-1`（Opus -1）
  > overlay→canonical の最重 scene 操作が `action_for` で nil。実行者は owner/editor に限定されるため -1。
  > 対象ファイル: `world-server/lib/world_web/plugs/rate_limit.ex`

- **overlay revision が canonical 更新でも 0 に戻らず、上限到達で部屋が詰む** `-1`（Opus -1）
  > packed revision の overlay 側が単調増加のみ。上限で takeover / dormant も失敗し復旧手段無し。到達は遠いが時限式。
  > 対象ファイル: `world-server/lib/world/rooms/internal.ex`

- **assets ローカル CI が永続 `_build` volume で初回 FAIL する** `-1`（双方 -1）
  > `redefining module` + `--warnings-as-errors`。テスト本体 240 は PASS。ボリューム掃除後は全緑。
  > 対象ファイル: `.scripts/ci.sh`

- **dash 肥大 / host 一枚岩再成長 / 必ず失敗する smoke 案内 / trust-boundary 欠行** `-2`（Opus 群を圧縮）
  > `dash.rs` 3779 行、`sim.rs`/`network_worker.rs` 各 ~3000 行、README の `smoke-rapier-host.sh`、trust-boundary に edit/sim_state/SpawnTicket/VCS 行無し。
  > 対象ファイル: `client/app/src/system_ui/dash.rs`, `client/host-server/src/sim.rs`, `README.md`, `.workspace/0_doc/architecture/trust-boundary.md`

---

## スコア感（採用マイナス）

| 帯 | 点数 |
|:---|---:|
| P0/P1 revoke・物理・流量・配信 | ≈ -12 |
| P2 門番抜け・衛生・CI | ≈ -6 |
| **合計概算** | **≈ -18** |

前回まとめ採用マイナス ≈ -23 から改善。旧 P0（takeover 過大権限・UI host 資格情報・handover・SceneBudget 等）はほぼ閉じ、穴は **social revoke の深さ・既定物理の計算量・assets RL・cell 本線の回復/E2E** に移った。
