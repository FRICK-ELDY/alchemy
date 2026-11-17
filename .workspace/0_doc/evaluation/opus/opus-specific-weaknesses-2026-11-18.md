# Alchemy — マイナス点詳細（第1評価者 Claude Opus 5.5 / 2026-11-18）

> 評価対象コミット: `57ca452`（main・PR #247 / x00d6 直後）。前回（自系統）`8833821` から 135 コミット（x009d〜x00d6 の 58 スライス）。
> 全項目は当日コードを直接読んで確認した。第2評価者（`gpt/`）の文書は当日・アーカイブとも参照していない。
> 改善計画（`archive/2026-11-12/improvement-plan-2026-11-12.md`）の「closed」注記は信じず、すべて対象ファイルを読み直して判定した。
> 前回からの継続項目には「継続」、一部解消は「部分解消」、今回新たに見つけたものには「新規」と付記する。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 改善余地あり。動作はするが設計・品質上の軽微な問題 |
| -2 | 重要な機能・設計の欠如。放置すると将来の拡張を阻害する |
| -3 | 設計上の明確な欠陥。バグ・クラッシュ・性能劣化を引き起こしうる |
| -4 | プロジェクトの価値命題を損なう重大な欠如。説明責任が果たせない |
| -5 | プロジェクトの根幹を揺るがす致命的な欠陥。存在しないに等しい |

| ブロック | 観点 | 小計 |
|:---|:---|---:|
| 運営側コア | auth-server | 0 |
| 運営側コア | assets-server | -2 |
| 運営側コア | world-server | -3 |
| 共通契約 | protocol/ | 0 |
| 共通契約 | client/shared-sim | -3 |
| ホスト端末 | client/host-server | -1 |
| ホスト端末 | client/app（HostSupervisor 含む） | -2 |
| ホスト端末 | launcher/ | 対象外（退役） |
| 転送・エッジ | relay-server / edge / relay-agent | 0 |
| 横断 | テスト戦略 | -2 |
| 横断 | 可観測性 | 0 |
| 横断 | エラーハンドリング | 0 |
| 横断 | 変更容易性・保守性 | 0 |
| 横断 | 開発者体験（DX） | -1 |
| 横断 | セキュリティ・配布可能性 | 0 |
| 横断 | プロジェクト全体設計 | -1 |
| | **合計** | **-15** |

---

## 技術評価層 — 運営側コア

### auth-server

（当日の新規マイナスなし。変更は `Auth.WorldNotify`（auth → world の relation_changed 通知）のみ。本番は `WORLD_INTERNAL_URL` 未設定で起動しない（`config/runtime.exs` L88-96）ため、設定漏れで黙って失効が止まる経路も無い。）

### assets-server

- **HTTP レート制限が一つも無いまま、重い新経路（VCS negotiate / push・SpawnTicket 発行）が増えた（新規）** `-2`
  > `assets-server/mix.exs` の deps（L30-47）に `alchemy_rate_limit` が無く、`lib/`・`config/` を `rate_limit|RateLimit|throttle` で検索しても 0 件。auth と world は操作別に IP × 主体の二軸バケットを持つが、コア 3 サービスのうち assets だけが入口の流量制御を持たない。
  > この 1 週間で assets に入った経路はどれも重い。`POST …/vcs/push` は body 最大 12 MiB、1 回で最大 256 commit・512 object を受け、各 object を常駐 Port（`Assets.VcsObject.Worker`、`PartitionSupervisor` で **N 本固定**・moduledoc L8-10「more-than-N parallelism は非保証」）に通して Rust で厳格デコードし、単一 TX で ref CAS する。`negotiate` は 64 KiB、`spawn_tickets` は blake3 ごとの Grant 照会と RS256 署名を行う。Quota は「保存容量」を抑えるが、**CPU と Port 占有時間は抑えない**。認証済みの 1 ユーザーが push を連打すれば Port プールを専有し、他ユーザーの push と Port 待ちのアップロード経路を飢えさせられる。
  > 前回の自分の強み（auth 項）で「`alchemy_rate_limit` を auth / world / assets で共有」と書いたが、`8833821` 時点でも assets は未導入だった。ここで訂正する。
  > 改善方針: リポジトリ内の path 依存 `shared/alchemy_rate_limit`（外部パッケージの新規追加ではない）を assets に入れ、`blob_put` / `vcs_push` / `vcs_negotiate` / `spawn_ticket` を操作別バケット（IP × `sub`）にする。push は「1 ユーザー同時 1 本」のセマフォも足すと Port プールの公平性が担保できる。world の `plugs/rate_limit.ex` と同形にすればテストもそのまま流用できる。
  > 対象ファイル: `assets-server/mix.exs`, `assets-server/lib/assets_web/router.ex`, `assets-server/lib/assets_web/controllers/vcs_controller.ex`, `assets-server/lib/assets_web/controllers/spawn_ticket_controller.ex`, `assets-server/lib/assets/vcs_object/worker.ex`

### world-server

- **kick は「鍵を回す」だけで、追い出した相手の再入室を止められない（部分解消）** `-1`
  > 閉じた部分: owner 限定の `POST /rooms/:id/room_key/rotate`（`router.ex` L33、rate limit は `room_visibility` バケット）と、auth からの `POST /internal/v1/relation_changed`（L40）→ `rotate_for_relation_change` が入り、friends 部屋でフレンドを外すと epoch が進む。改善計画 P1-1 の完了条件（旧鍵で新 epoch が読めない）は満たす。
  > 残件: (1) **拒否リストが無い**（world `lib/` を `ban|deny_subs|kicked` で検索して 0 件）。public / link 部屋では、追い出された本人がそのまま `POST …/tokens` を叩けば新 epoch の鍵を受け取れるので、kick が「数秒の切断」にしかならない。(2) client に kick の導線が無い（`client/app/src` から `room_key/rotate` を呼ぶ箇所 0 件）。(3) `Auth.WorldNotify` は fire-and-forget で再試行キューを持たない（`world_notify.ex` L8-9 の非保証）。world が一時的に落ちていた間のフレンド解除は、24h 定期ローテまで効かない。
  > 改善方針: Room 行に owner 管理の `denied_subs` を持ち、Room Token 発行・takeover・handover accept の 3 箇所で拒否する。kick は「deny 追加 + 鍵ローテ」を 1 回の CAS で行う。relation_changed は auth 側に outbox テーブルを置いて再送する。
  > 対象ファイル: `world-server/lib/world/rooms/room_key.ex`, `world-server/lib/world_web/router.ex`, `auth-server/lib/auth/world_notify.ex`

- **`POST /rooms/:id/scene/save` だけが rate limit の対象外（新規）** `-1`
  > `plugs/rate_limit.ex` の `action_for/1` は `GET scene`（L117）・`scene/entries` の POST / DELETE（L120-130）・`overlay/entries` の POST / DELETE（L132-142）を `:host_scene` に割り当てるが、x00b7 で増えた `POST …/scene/save`（`router.ex` L58）に当たる節が無く、L150 の `action_for(_conn), do: nil` に落ちて**無制限**になる。save は overlay を canonical に畳み込み、SceneBudget で最大 1 MiB の JSON を encode し、packed CAS と overlay クリアを行う、scene 系で最も重い操作。PoP と `ensure_scene_writer`（`room_controller.ex` L637 等）で実行者は owner / editor に絞られるので -1 に留めるが、x0084 で決めた「host-token HTTP はすべて操作別バケット」という方針に対する抜けで、editor の session 鍵が漏れた場合の増幅面になる。
  > 改善方針: `scene/save` を `:host_scene`（または上限を低くした `:host_save`）に割り当てる節を足し、「save を連打すると 429」の否定系テストを入れる。
  > 対象ファイル: `world-server/lib/world_web/plugs/rate_limit.ex`（L117-150）, `world-server/lib/world_web/router.ex`（L58）

- **overlay revision が canonical 更新でも 0 に戻らず、上限 1,000,000 に達した部屋は takeover / dormant 遷移が止まり、復旧手段も無い（新規）** `-1`
  > packed revision は `canonical × 1_000_000 + overlay`（`rooms/internal.ex` L11）。overlay 側は append / delete のたびと、host 交代・dormant でのクリアのたびに +1 され（L43-53）、canonical が進んでも**リセットされない**。上限に達すると `overlay_clear_headroom_sql/0`（L55-57）が UPDATE を 0 行にし、`miss_after_overlay_clear/1` が `:invalid_revision` を返す（L91-98）。GenRev を巻き戻さない fail-closed 自体は正しいが、その部屋は overlay 書き込みも、overlay クリアを伴う takeover / dormant 遷移も永久に失敗し続け、管理操作も無い。x00b9 で spawn が live CAS を使わなくなったため到達は遠いので -1 とするが、長寿命の常設ワールドほど確実に近づく「時限式」の欠陥。
  > 改善方針: canonical を進める UPDATE（save / scene CAS）で overlay revision を 0 に戻す。canonical が +1 されるので packed 値は単調のまま保てる。上限直前の境界テストも足す。
  > 対象ファイル: `world-server/lib/world/rooms/internal.ex`（L11, L43-98）, `world-server/lib/world/rooms/scene.ex`

---

## 技術評価層 — 共通契約層

### protocol/

（当日の新規マイナスなし。`PROTOCOL_PIN` は `alchemy-protocol/0.24.0-x00b8`（`lib.rs` L80）に上がり、新しい proto（`sim_state` / `edit`）と `vcs_object` も pin とテストを伴って入っている。）

### client/shared-sim

- **既定物理経路の LBVH 構築が O(n²) で、しかも毎サブステップ（60Hz × 4）作り直される（新規）** `-2`
  > `FlatLbvh::build_from_sheet`（`spatial.rs` L169-184）は morton コードを radix sort する（ここは良い）が、葉ノードを作るとき `aabb_of` クロージャが `leaf_aabbs.iter().find(|(r, _)| *r == row)` で**葉ごとに線形探索**する（L191-197、呼び出しは L202-211）。葉数 n に対して O(n²)。
  > これが既定経路の毎サブステップで走る。`XpbdWorld` の既定 `substeps: 4`（`xpbd.rs` L36）、`step` はサブステップごとに `scene_bounds` があれば LBVH を作り直す（L429-448）、host の `step_dt` は常に `Some(self.scene_bounds)` を渡す（`host-server/src/sim.rs` L1238）、`ALCHEMY_SIM_BACKEND` の既定は `EpochXpbd`（`sim_backend.rs`）。MVCC 側の `mvcc/adapter.rs` も同じ LBVH を包むので、どちらのバックエンドでも同じ。
  > world 側の SceneBudget は 1 部屋 4096 entries を許す（`scene_budget.ex`）。n = 4096 なら 1 回の構築で約 840 万回の比較、それを秒間 240 回で約 20 億回。world が受理した scene を host が 60Hz で回せない、という層間の不整合になる。radix sort まで入れておきながら AABB の引き当てで台無しにしている、典型的な「並べ替えたのに索引を捨てた」バグで、プロジェクトの DOD 規則（データの並び・アクセスパターン先行）に正面から反する。
  > 改善方針: radix sort の対象を `(code, leaf_index)` にして AABB を添字で引く（O(n)）。1 tick 内でトポロジは変わらないので、サブステップ 2 回目以降は再構築ではなく refit にする。shared-sim に n = 4096 の時間計測テスト（`#[ignore]` で CI 任意実行）を置き、SceneBudget 上限と物理予算を同じ数字で縛る。
  > 対象ファイル: `client/shared-sim/src/spatial.rs`（L169-243）, `client/shared-sim/src/xpbd.rs`（L36, L413-448）, `client/host-server/src/sim.rs`（L1220-1275）

- **既定経路で毎 tick 全シート clone 3 回と、サブステップごとの一時確保が残る（継続）** `-1`
  > `step_dt` は `let start = self.world.curr.clone();`（`sim.rs` L1231）、`self.snapshots.publish(start, self.world.curr.clone())`（L1265）、`commit_prev_from_curr()`（L1272 → `xpbd.rs` L45-46 の `self.prev = self.curr.clone()`）で、tick ごとに `EpochSheet` 全体を 3 回複製する。`predict_integrate` などは毎回 `iter_active_rows().collect()` で `Vec<u32>` を作り（`xpbd.rs` L55, L336）、彩色は色ごとに `HashSet::new()`（L101）。
  > 改善計画 P1-3 の「Phase 1 closed」はコード上も正しい（MVCC page 経路に `arena_cow_1k_entities_500_ticks_page_alloc_bounded`、page 確保 ≤ 2 / tick のテストがある）。しかし本番の既定バックエンドは `EpochXpbd` で、`sim.rs` 冒頭の非保証にも「zero-alloc sheet↔page」と書かれている。確保ゼロを証明した経路と、実際に走っている経路が別、という状態。
  > 改善方針: `clone_from` / `std::mem::swap` で永続バッファを使い回し、snapshot は二面バッファの交換にする。`XpbdWorld` には既に「サブステップ間で再利用する scratch」（L24）があるので、rows / 彩色マーカー（epoch 印付き `Vec<u32>`）もそこへ移す。カウンタアロケータのテストを `EpochXpbd` 経路にも当てる。
  > 対象ファイル: `client/host-server/src/sim.rs`（L1220-1275）, `client/shared-sim/src/xpbd.rs`（L24, L45-46, L53-55, L95-101, L336）

---

## 技術評価層 — ホストユーザー端末

### client/host-server

- **`sim.rs`（3253 行）と `network_worker.rs`（2790 行）が新しい一枚岩になった（新規）** `-1`
  > x0099 で `main()` を `HostLoop` に分けて一枚岩を解消したが、この 1 週間で `sim.rs` に 2 バックエンド（EpochXpbd / MVCC page）・Grab・接触・`sim_state` からの再開・テスト群が、`network_worker.rs` に Room frame・cell / lod の soft-put・LRU キャッシュ・dirty delta・冗長送信が集まった。とくに cell / lod の「どのセルに何を put / remove するか」は Spatial Interest の中核ロジックなのに、Zenoh put と同じファイルに埋まっていて純関数として単体テストしにくい。x00d6 で world の `rooms.ex` を分割したのと同じ処置が host 側にも要る。
  > 改善方針: `sim/{epoch,mvcc,grab,resume}.rs` に分け、`network_worker` は「セル集合の差分 → put / remove 計画」を純関数（入力: 前 tick と今 tick の占有セル、出力: 計画）に切り出してから `publish/{room,cell,lod}.rs` に分割する。
  > 対象ファイル: `client/host-server/src/sim.rs`, `client/host-server/src/network_worker.rs`

### client/app

- **`dash.rs` が 3779 行のまま（部分解消）** `-1`
  > 閉じた部分: `desktop.rs`（3805 行）は x00d0 で `surface/desktop/{mod.rs 1731, session_flow.rs 1981, inventory_flow.rs 949, hotswitch_flow.rs}` に分かれ、`system_ui` は `UiAction`（`system_ui/mod.rs` L1453）を返すだけになった。改善計画 P2-1 が狙った「イベント → コマンド」の分離はコード上で成立している。
  > 残件: `system_ui/dash.rs` は 3604 → 3779 行とさらに伸びた。中身は UI のみだが、Worlds・Room タブ（x00b6）・Inventory・設定などのタブがすべて 1 ファイルにある。
  > 改善方針: タブ単位で `dash/{worlds,room,inventory,settings}.rs` に分け、レイアウト計算を純関数にしてテストする。
  > 対象ファイル: `client/app/src/system_ui/dash.rs`

- **既定にした cell 配信に、取りこぼしからの回復手段が無い（新規）** `-1`
  > x00c5 で client の既定購読を Room `…/frame` から近傍 cell（Moore ≤ 27 キー）に切り替えた。`bus/frame.rs` の非保証（L37-43）は正直に書いてある: 「host の cell soft-put が失敗したときの回復（既定では Room frame を購読していないので埋め戻しが無い）」「cell remove を取りこぼしたときの AOI 不変な遠方ゴースト（定期 prune 無し。次の enter か gap→snapshot でしか消えない）」。
  > Zenoh の put は best effort で、sleep 中の entity は dirty delta を出さない。upsert を 1 回落とすと、その entity は**次に動くまで古い姿勢のまま**、remove を落とすと遠方のゴーストが残る。旧既定の Room frame には tick gap → snapshot の再同期があったが、cell 経路は lod が `last_frame_tick` を進めない（L28-29）など gap 検知の網が粗い。目標機能ではなく「既定の本線」の性質なので減点する。
  > 改善方針: host が占有セルごとに低頻度（例: 1Hz）のキーフレームを soft-put する、またはセルごとの連番を付けて client が欠番で snapshot を取り直す。client 側は「Moore 外かつ最終更新から T 秒」の entity を定期的に prune する。
  > 対象ファイル: `client/app/src/bus/frame.rs`（L20-45）, `client/host-server/src/network_worker.rs`

### launcher/

（git 管理外・評価対象外。）

---

## 技術評価層 — 転送・公開エッジ

### relay-server / edge / relay-agent

（当日の新規マイナスなし。前回 -1 の `curl` 子プロセスと lock `expect` は x00d3 で closed。WAN ACL は cell / lod / edit / sim_state を明示キーで追加し、`frame/**` に広げない方針もコメントで固定している。）

---

## 横断評価層

### テスト戦略

- **ローカル CI の assets が初回だけ落ちる（原因を変えて再発・部分解消）** `-1`
  > x00d1 で CI 用の名前付きボリューム（`alchemy-ci-assets-deps` / `alchemy-ci-assets-build`）に分離した（`ci.sh` L240-246）。開発用コンテナとの共有による前回の credo クラッシュは再発していない。
  > しかし当日の初回 `ci.sh all` で、`mix compile --warnings-as-errors` が `warning: redefining module Assets.TestPlaceReadHelpers / Assets.DataCase / AssetsWeb.ConnCase / Assets.CasTmp / Assets.TestAuthHelpers (current version loaded from _build/test/lib/assets/ebin/…)` で落ちた。`ci.sh assets` の単独再実行では 240 tests, 0 failures で通過。CI 専用ボリュームが `_build/test` を実行間で持ち越すため、`test/support`（`mix.exs` L27 の `elixirc_paths(:test)`）に変更が入った直後の初回だけ、古い beam が読み込まれた状態で再コンパイルされる、と推定する（`test/test_helper.exs` はこの範囲で +77 行）。GitHub Actions は毎回新しいランナーなので影響はローカルのみ。
  > 改善方針: CI 用ボリュームを匿名ボリューム（`--rm` で消える）にするか、`-Werror` コンパイルの前に `mix clean --only assets` を入れる。
  > 対象ファイル: `.scripts/ci.sh`（L232-248）, `assets-server/mix.exs`（L27）

- **新しいワイヤ経路（cell / lod / edit / sim_state）と SpawnTicket が E2E に一本も入っていない（新規）** `-1`
  > WAN の zenohd ACL は本番用・E2E 用の両方で `frame/cell/*`・`frame/lod/*`・`edit`・`sim_state` を明示的に足した（`relay-server/config.wan.json5` / `config.wan.e2e.json5`）。ところが `.scripts/smoke-compose-wan-e2e.sh`・`docker-compose.e2e.yml`・`.github/workflows/e2e-compose.yml` はこの範囲で無変更、`client/e2e-probe` も `..Default::default()` の 2 行だけ。WAN E2E は今も Room `…/frame` の配送を確かめているが、**既定の client はもうそのキーを購読していない**。cell キーの ACL を 1 文字誤れば、既定の本線が WAN で全滅しても CI は緑のまま通る（x0067 の trace で `…/input` の egress ACL 漏れを見つけたのと同じ種類の事故）。
  > 前回は x0087 で「新しい保証は否定系 E2E で確認する」という完了条件を epoch について守っていたが、今回の 58 スライスではユニット / 統合テストに留まった。
  > 改善方針: e2e-probe が自分の Moore セルの `frame_cell_key` を購読し、WAN 経由で封緘済み frame が届くことを確かめる。否定系として「guest による `frame/cell/*` への put が届かない」「guest は `sim_state` を購読できない」「同じ `(jti, blake3)` の SpawnTicket を 2 回使うと 403」をヘルメティック E2E に足す。
  > 対象ファイル: `.scripts/smoke-compose-wan-e2e.sh`, `client/e2e-probe/src/main.rs`, `.github/workflows/e2e-compose.yml`, `relay-server/config.wan.e2e.json5`

### 可観測性

（当日の新規マイナスなし。）

### エラーハンドリング

（当日の新規マイナスなし。）

### 変更容易性・保守性

（当日の新規マイナスなし。肥大は host-server / client/app 側で個別に計上。）

### 開発者体験（DX）

- **README が、必ず失敗する smoke（`smoke-rapier-host.sh`）を案内している（部分解消）** `-1`
  > 閉じた部分: x00d1 で overview / client.md / README 本文の Rapier・launcher 表記は XPBD / HostSupervisor に直った。
  > 残件: `README.md` L117 が `bash .scripts/smoke-rapier-host.sh # host 60Hz / net 30Hz（zenohd 必須・スクリプト名は歴史的）` と案内している。スクリプトは `"Rapier world ready"` を grep する（L24）が、host は今 `"XPBD world ready …"` を出す（`host-server/src/main.rs` L1138）。ほかのバナー（dual-tick・grabbable boxes）も現行ログと一致しないので、手順どおりに実行すると**必ず FAIL** する。ci.sh / CI からは呼ばれていないので気づかれない。x0089 で同種の smoke を一掃した後に 1 本残った形。
  > 改善方針: 削除して README の行を `make ci-e2e` に置き換えるか、現行バナーに合わせて CI に載せる。
  > 対象ファイル: `README.md`（L117）, `.scripts/smoke-rapier-host.sh`

### セキュリティ・配布可能性

（当日の新規マイナスなし。assets の流量制御は assets-server ブロックで計上。）

### プロジェクト全体設計

- **trust-boundary.md に `…/edit`・`…/sim_state`・SpawnTicket・VCS push の行が無い（新規）** `-1`
  > この範囲の trust-boundary.md の差分は +7 行で、cell / lod の行（L45-46）、world scene 行への HostDelegation・PoP nonce の追記、HostSupervisor の AUTH_PUSH を足した。そこは良い。しかし `/edit|sim_state|SpawnTicket|VCS` で検索すると 0 件。
  > `…/edit` は guest が put できる**新しい入口**（ACL `guest-put-input` に追加）で、コード側は封筒サイズ上限 → Room AEAD → session Ed25519 → Dual-auth → 共有 control rate → `control_seq` と丁寧に守っている（`host-server/src/edit.rs` L90-104, L233-263）。それなのに信頼境界の正本にはその防御が書かれていない。SpawnTicket は「assets が発行し world が 1 回だけ消費する」サービス間の権限証票で、VCS push は Port で検証した object を owner だけが ref 更新できる入口。どちらも trust-boundary に無い。
  > 前回、この文書の継続保守を +4 と評価した。今回のスライスは各自の保証表（スライス文書・モジュール冒頭）は書いているが、横断の正本への反映が漏れている。
  > 改善方針: `…/edit`（guest put・AEAD 前の封筒上限・Dual-auth・tip を巻き戻さない Commit）、`…/sim_state`（host ↔ host のみ・guest 購読 deny）、SpawnTicket（`aud`・`(jti, blake3)` 一回限り・CAS 競合時のみ解放）、assets VCS push（owner のみ・Port 検証・件数上限・rate limit 無し）の 4 行を足す。スライス完了条件に「trust-boundary.md の該当行」を入れる。
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`
