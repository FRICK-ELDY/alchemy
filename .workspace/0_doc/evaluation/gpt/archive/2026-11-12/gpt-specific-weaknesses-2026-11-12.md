# Alchemy 第2評価者 — 具体的な弱み（2026-11-12）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 改善余地あり。動作はするが設計・品質上の軽微な問題 |
| -2 | 重要な機能・設計の欠如。放置すると将来の拡張を阻害する |
| -3 | 設計上の明確な欠陥。バグ・クラッシュ・性能劣化を引き起こしうる |
| -4 | プロジェクトの価値命題を損なう重大な欠如 |
| -5 | プロジェクトの根幹を揺るがす致命的欠陥 |

## 運営側コア

### world-server — ホスト権限・Room 寿命

- **可視 Room の任意参加者が dormant Room の計算ホスト権限を取得できる** `-4`
  > `takeover/2` の CAS は dormant / lease 失効だけを条件にし、HTTP 境界も `RoomAccess.authorize/4`、つまり「見える／入れる」ことしか検査しない。実際の controller test は public Room を所有者でない `guest_a` が takeover して `active_host_sub` になることを成功条件にしている。取得後は host Room Token と PoP 鍵を得て canonical scene の append/delete、attestation、seal を正規に実行できるため、「見る権利」と「定義・実行を引き受ける権利」が混ざっている。Untrusted host 前提でも、委譲 capability なしに canonical 定義変更権まで渡すのは過大権限である。
  > 改善方針: owner が署名した短命 HostDelegation、または world 台帳の `eligible_host_subs` / `can_be_host` を takeover CAS 条件に加える。visibility は入室判定にだけ使い、ホスト昇格判定と分離する。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex:569-591`, `world-server/lib/world/rooms.ex:933-969`, `world-server/test/world_web/controllers/room_controller_test.exs:1842-1863`

- **Room 鍵ローテは入ったが、関係解除・kick 相当の即時 revoke は未完** `-2`
  > `room_key_epoch`、visibility 縮小、owner 手動、24時間周期の CAS ローテは実装済みで、前回の「部屋寿命で固定」は大きく改善した。一方 `Rooms` 自身が friendship change では回転しないと明記し、membership / kick 台帳もない。Friends Room で friendship を解除しても、過去参加者は次の定期ローテまで旧鍵を保持し、WAN wildcard 購読経由で復号できる。Room Token TTL 5分より最長約24時間長い認可残留になる。
  > 改善方針: auth の relation revision または world membership revoke イベントから epoch CAS を呼び、旧主体を grace 配布対象から除外する。明示 kick API を導入する場合も同じトランザクション境界で回転する。
  > 対象ファイル: `world-server/lib/world/rooms.ex:20-24,499-550,1540-1617`, `world-server/lib/world/rooms/room_key_grace.ex:5-22`, `relay-server/config.wan.json5`

- **canonical scene の構造上限がなく、有効 host が永続 JSON を肥大化できる** `-2`
  > append は blake3 形式・重複・revision CAS を検査するが、entries 件数、scene JSON byte、owner/Room ごとの配置上限を検査しない。host-token rate limit は時間当たりの増幅を抑えるだけで、長時間の固有 hash 追記を止めない。肥大した scene は SQLite RMW、seal hash、Manifest encode、client decode 上限へ波及し、正本が client の受理上限を超える自己 DoS を起こせる。
  > 改善方針: world の定義境界で `MAX_SCENE_ENTRIES` と canonical JSON byte 上限を原子的に検査し、protocol decode 上限以下に固定する。上限・競合・削除後再追加を統合テストする。
  > 対象ファイル: `world-server/lib/world/rooms.ex:188-218,310-374`, `world-server/lib/world_web/controllers/room_controller.ex:795-852`

## 共通契約・シミュレーション

### client/shared-sim — ホットパス

- **60Hz ホットパスで EpochSheet 全体 clone と複数一時確保を回している** `-3`
  > SoA、世代付き row、ArcSwap の方向は良いが、host 実経路は tick ごとに `start = curr.clone()`、publish 時に `curr.clone()`、最後に `prev = curr.clone()` を行う。汎用 `FrameBridge::tick` も Prev/Curr を clone して publish 後さらに commit clone する。XPBD は substep ごとに active row `Vec`、constraint coloring の `to_vec` / `HashSet` / 複数 `Vec`、LBVH build の一時領域を確保し、`build_lbvh_from_sorted` は各 leaf AABB を線形探索するため構築部分が O(n²) になる。現状規模では通るが、設計が掲げる「フレーム間でアロケーションを回さない」と一致せず、entity 増加時の 60Hz 保証をまだ持てない。
  > 改善方針: Prev/Curr 所有バッファを swap し、公開 face は世代固定プールまたは dirty page COW にする。color/LBVH scratch を world に常駐させ、row→AABB の直接配列参照へ変える。1k/10k entity の alloc count・p95 tick benchmark を CI 回帰に置く。
  > 対象ファイル: `client/host-server/src/sim.rs:600-634`, `client/shared-sim/src/bridge.rs:120-139`, `client/shared-sim/src/xpbd.rs:45,94-120,372-425`, `client/shared-sim/src/spatial.rs:169-198`

## ホストユーザー端末

### HostSupervisor / host-server

- **通常 UI ログインで起動した host は約15〜20分後に資格情報更新不能となる** `-3`
  > auth access JWT は 900秒だが、HostSupervisor は UI の access token だけを子へ `ALCHEMY_AUTH_TOKEN` として渡し、refresh token を渡さない。host はこの env token を `expires_in=0`（長命扱い）として auth refresh をスケジュールせず、期限切れ後の Room Token 再発行が失敗し、最後の host Token / seal hard expiryで配信停止する。`AUTH_USER`/password を親環境に置く回避は開発運用向けで、通常 UI ログインという主経路の永続ホスト保証にならない。
  > 改善方針: 親 client を資格情報 broker にし、localhost IPC で短命 host capability / 更新済み auth token を渡す。refresh token や password を子 env に複製せず、親が refresh と revoke を一元管理する。
  > 対象ファイル: `auth-server/config/config.exs:19`, `client/app/src/host_supervisor.rs:23-33,507-528`, `client/host-server/src/room_token/credential_chain.rs:180-208,665-696`

- **Graceful Handover が opt-in を見ず「最初の Presence peer」を5分間指名する** `-2`
  > client 終了時は `first_presence_peer_sub()` をそのまま `target_sub` に送り、world は owner/active-host なら任意 sub を active にして lease を300秒付与する。対象の `server_operator` / `can_be_host` は world に証明されず、対象 client も operator OFF なら自動起動しない。その間 Room は active 扱いのまま実 host がなく、ほかの候補の takeover は lease 失効まで409になる。
  > 改善方針: Presence ではなく world が検証した HostCandidate lease から選び、候補の明示 accept 後に二段階 CAS で切り替える。候補不在時は即 dormant にして takeover を可能にする。
  > 対象ファイル: `client/app/src/scene.rs:429-435`, `client/app/src/surface/desktop.rs:562-593`, `world-server/lib/world/rooms.ex:900-928,1069-1089`

### client/app — 資格情報・鍵更新 UX

- **7日 refresh token を平文 JSON に保存する** `-2`
  > `%APPDATA%/Alchemy/onboarding.json`（または `~/.config`）へ refresh token を `fs::write` し、暗号化・OS credential vault・Unix 0600 強制を行わない。ログには出さないが、同一端末上の別プロセス、緩いバックアップ同期、別ユーザーから読める権限設定では7日セッションを奪取できる。配布可能なデスクトップ client としては bearer secret の保存境界が弱い。
  > 改善方針: Windows Credential Manager/DPAPI、macOS Keychain、Secret Service を抽象化し、prefs には opaque key id のみ保存する。vault 非対応環境では session-only を既定にし、Unix は少なくとも0600 + atomic replaceにする。
  > 対象ファイル: `client/app/src/operator_prefs.rs:1-9,145-164,237-258`

- **定期 Room key rotation が全参加者の手動 Rejoin を要求し、XR 単独 UX がない** `-1`
  > epoch 前進時は新鍵を live AEAD に適用せず KeyStale とし、Desktop の Rejoin でのみ commit する。24時間周期ローテでも同じため、長時間イベントは設計上必ず一度中断する。Desktop は明示 overlay を持つが、architecture 文書自身が XR の KeyStale overlay / Rejoin HTTP を非保証としている。
  > 改善方針: 旧鍵受理 grace 中に新 Token と bus generation を準備し、同一 session で current key を原子的に切り替える。失敗時だけ明示 Rejoin にフォールバックし、XR にも同等操作を出す。
  > 対象ファイル: `client/app/src/scene.rs:77-93,852-913`, `client/app/src/system_ui/layout.rs:150`, `.workspace/0_doc/architecture/client.md:43`

**弱み合計: -19**
