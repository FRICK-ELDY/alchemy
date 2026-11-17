# Alchemy 第2評価者 — 具体的な弱み（2026-11-18）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 改善余地あり。動作はするが設計・品質上の軽微な問題 |
| -2 | 重要な機能・設計の欠如。放置すると将来の拡張を阻害する |
| -3 | 設計上の明確な欠陥。バグ・クラッシュ・性能劣化を引き起こしうる |
| -4 | プロジェクトの価値命題を損なう重大な欠如 |
| -5 | プロジェクトの根幹を揺るがす致命的欠陥 |

## 運営側コア

### auth-server / world-server — social revoke

- **followers 系と推移関係の Room 鍵 revoke が不完全で、通知も永続化されない** `-3`
  > `unfollow/2` は accepted friendship があった場合しか `relation_changed` を送らないため、通常の follower が unfollow しても `followers` Room の鍵は回転しない。また world は通知された二者が owner の Room だけを走査するため、第三者 owner の `friends_plus` / `followers_plus` から外れた viewer の鍵も回転しない。さらに通知は fire-and-forget で retry/outbox がなく、world 一時停止時は取りこぼす。該当主体は次の定期回転（既定24時間）まで旧 epoch を復号できる。改善方針は、全 unfollow を durable outbox に記録し、関係 revision または影響 owner 集合を world 側で再評価して epoch CAS すること。
  > 対象ファイル: `auth-server/lib/auth/social.ex:41-67,160-170`, `auth-server/lib/auth/world_notify.ex:1-91`, `world-server/lib/world/rooms/room_key.ex:50-89`

## 共通契約・シミュレーション

### client/shared-sim / host-server — 60Hz ホットパス

- **MVCC ページ経路を追加しても既定 host は全 EpochSheet clone を継続する** `-2`
  > `ALCHEMY_SIM_BACKEND` 未設定時は `EpochXpbd` で、各 tick に `start = curr.clone()`、publish 時の `curr.clone()`、最後の `commit_prev_from_curr()` が残る。`MvccXpbd` も現時点では Epoch sheet と単一 page の同期ブリッジを通り、tick 冒頭の `start` clone と snapshot clone を共有する。ページ pool の「page alloc ≤2」は実証されたが、1k entity の実 host 全体で zero-allocation / p95 16.6ms を保証したことにはならない。改善方針は、既定経路の buffer swap/COW 化と allocator 計測付き host benchmark である。
  > 対象ファイル: `client/host-server/src/sim_backend.rs:1-50`, `client/host-server/src/sim.rs:1220-1274`, `client/shared-sim/src/mvcc/adapter.rs:129-159`

## 横断評価層

### DX・ローカル CI

- **assets の永続 `_build` volume で warnings-as-errors が再実行不能になる** `-1`
  > `.scripts/ci.sh` は host 開発 volume とは分離したが、CI 用 named volume 自体は再利用する。今回、既存 `alchemy-ci-assets-build` に対する `mix compile --warnings-as-errors` が test support 5 module の `redefining module` warning で失敗した。同じ状態で `mix test` は 240 tests PASS したためコード機能障害ではないが、単一ローカル CI 入口の反復可能性を損ねる。改善方針は run 単位 build path、または compile 前の限定的な test-support consolidation 清掃である。
  > 対象ファイル: `.scripts/ci.sh:230-249`, `assets-server/test/support/`

**弱み合計: -6**
