# Alchemy — マイナス点詳細（第1評価者 Claude Opus 5.5 / 2026-11-12）

> 評価対象コミット: `8833821`（main・PR #187 / x009c 直後）。前回まとめ時点 `c491016`（2026-11-05）から 122 コミット（x007c〜x009c、launcher 退役、HostSupervisor、Zero-Base spatial / XPBD、Room 鍵 epoch、制御面 PoP、scene CAS 等）。
> 全項目は当日コードを直接読んで確認した。第2評価者（`gpt/`）の文書は当日・アーカイブとも参照していない。
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
| 運営側コア | assets-server | 0 |
| 運営側コア | world-server | -8 |
| 共通契約 | protocol/ | 0 |
| 共通契約 | client/shared-sim | -1 |
| ホスト端末 | client/host-server | -2 |
| ホスト端末 | client/app（HostSupervisor 含む） | -5 |
| ホスト端末 | launcher/ | 対象外（退役） |
| 転送・エッジ | relay-server / edge / relay-agent | -1 |
| 横断 | テスト戦略 | -1 |
| 横断 | 可観測性 | 0 |
| 横断 | エラーハンドリング | 0 |
| 横断 | 変更容易性・保守性 | 0 |
| 横断 | 開発者体験（DX） | 0 |
| 横断 | セキュリティ・配布可能性 | 0 |
| 横断 | プロジェクト全体設計 | -1 |
| | **合計** | **-19** |

---

## 技術評価層 — 運営側コア

### auth-server

（当日の新規マイナスなし。x008b で `/ready`（Repo・Keys）が加わったのみで、既存の強みは維持。）

### assets-server

（当日の新規マイナスなし。前回の 3 件 — BLAKE3 Port 直列・mime 先着・Origin Cert の内容非束縛 — は x008d / x008b / x0096 でコード上 closed。）

### world-server

- **Takeover した第三者が、オーナーの永続 canonical scene を書き換えられる（定義と実行の逆転・新規）** `-3`
  > x0080 で `owner_sub`（永続オーナー）と `active_host_sub`（現計算ホスト）を分けたのは良い設計だが、**scene の書き込み権限が「現計算ホスト」に付いている**。`POST /rooms/:id/takeover` は `RoomAccess.authorize`（公開範囲 ACL）と CAS（`dormant` または lease 切れ）だけを見る（`room_controller.ex` L569-575、`rooms.ex` L942-969）。つまり public の部屋なら、登録ユーザーは誰でも、部屋が `dormant`（誰もホストしていない平常状態）の間に takeover できる。成功すると `authorize_host_role/2` が `active_host_sub == sub` で host Room Token を出し（`rooms.ex` L809-822）、そのユーザーは自分の session 鍵で PoP を付けて `POST/DELETE /rooms/:id/scene/entries` を呼べる（`room_controller.ex` L798-811, L896-906 は `ensure_active_host` しか見ない）。scene は Room 行の JSON で、`cas_write_dormant` / lease sweep でも巻き戻らない（`rooms.ex` L980-1000, L1093-1111）。
  > 結果として、**他人の公開ワールドのメッシュを恒久的に削除し、任意の blake3 を差し込める**（差し込んだ hash は次の seal で PlaceHashCache に入り、以後その部屋の参加者に PlaceRead で配られる）。x0080 スライスの保証表も「visibility 変更・部屋削除相当は owner のみ」と書く一方で、scene 書き込みには触れていない。実装ルールの「server = 定義、client/host = 定義に基づく実行」に照らすと、実行者（host）が定義（canonical scene）を書いている。
  > 改善方針: scene の永続書き込み（append/delete）は `owner_sub`（または owner が明示的に付与した editor）に限り、takeover host は「セッション内の一時 entity」だけを持てるよう分ける（例: `scene` と `session_overlay` を分離し、dormant で overlay を破棄）。最低限、`create_scene_entry` / `delete_scene_entry` に `room.owner_sub == claims["sub"] or claims["sub"] in editors` を足し、否定系テスト（非オーナー takeover → 403）を入れる。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`（L569-598, L798-906）, `world-server/lib/world/rooms.ex`（L801-822, L932-969, L1093-1111）, `.workspace/roadmap/slices/x0080-host-handover-and-takeover.md`

- **Room 鍵 epoch は入ったが、kick API が無く、フレンド解除ではローテしない（部分解消）** `-1`
  > x007e で epoch 化は本物になった（AAD に epoch、CAS ローテ、grace、24h 定期ローテ）。ただし owner 主導の `rotate_room_key/2`（`rooms.ex` L536-550、moduledoc に「kick / manual revoke entry」）には HTTP ルートが無く（`router.ex` L16-43 に無い）、UI からも呼べない。ローテが起きるのは visibility の **縮小**と 24h 定期だけなので、friends 限定の部屋でフレンドを外しても、その人は最長 24h、WAN guest のワイルドカード購読で平文を読み続けられる。`RoomKeyGrace` は ETS のみで、world 再起動後に host の prev が消える点も保証表どおりだが運用上の穴。
  > 改善方針: `POST /rooms/:id/room_key/rotate`（owner のみ・rate limited）を足し、friendship 変更イベント（auth → world）でもローテを打つ。kick UI はそれを呼ぶだけにする。
  > 対象ファイル: `world-server/lib/world/rooms.ex`（L536-550）, `world-server/lib/world_web/router.ex`, `world-server/lib/world/rooms/room_key_grace.ex`

- **epoch の配布がポーリング（host・全 guest が 15 秒ごとに Room Token を再発行）で、world 負荷と rate limit を食う（新規）** `-1`
  > host は `ROOM_KEY_SYNC_INTERVAL = 15s`（`credential_worker.rs` L30-32）、guest も約 15 秒の epoch probe（`client/app/src/room.rs` L17-20「world has no epoch-only API — issued jti is discarded」）で `POST /tokens` を叩く。毎回 Room Token と Allocation Token の 2 本の RS256 署名・ログ・メトリクスが走り、捨てられる。`room_token` の IP 上限は 60/分（`world-server/config/config.exs` L55-57）なので、同一 NAT 配下 15 人で probe だけで上限に届く。overview の「15 人集会」がそのまま上限に当たる設計。
  > 改善方針: `GET /rooms/:id/room_key_epoch`（Room Token 認可・署名不要・軽量）を足し、epoch が変わったときだけ Token を取り直す。将来は host が Zenoh で「epoch bumped」ヒントを流し（署名付き Presence 等）、ポーリング間隔を伸ばす。
  > 対象ファイル: `client/host-server/src/room_token/credential_worker.rs`, `client/app/src/room.rs`, `world-server/config/config.exs`

- **`rooms.ex`（1734 行）と `room_controller.ex`（1476 行）が新しい一枚岩になり、エラー→HTTP 変換が各アクションに手書きで重複（新規）** `-1`
  > Room 定義・visibility・Room 鍵ローテ・host lease / handover / takeover・scene CAS・catalog・owner quota が 1 モジュールに同居する。controller は `create_host_key` / `show_scene` / `create_scene_entry` / `delete_scene_entry` / `create_manifest_seal` がそれぞれ 20〜40 行の同型 `else` 節（`:pop_required` / `:not_host_token` / `:room_mismatch` / `:forbidden_host` / `:conflict` …）を持つ（L657-711, L746-789, L813-887 等）。「保証の単位でファイルを分ける」というプロジェクト原則に反し、新しい host API を足すたびに同じ表をコピーする構造。
  > 改善方針: Phoenix の `action_fallback` + `FallbackController` でエラー表を一箇所に寄せ、`Rooms` を `Rooms.Lease` / `Rooms.RoomKey` / `Rooms.Scene` / `Rooms.Catalog` に分割する。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

- **軽微群（catalog の rate limit 無し・scene append が Grant を見ない・HTTP PoP の skew 内リプレイ）（新規）** `-2`
  > (1) `GET /rooms`（catalog、1 要求で owned 200 件 + public 候補を ACL 評価）と `GET /rooms/:id` は `action_for/1` に無く無制限（`plugs/rate_limit.ex` L59-103）。(2) `append_scene_entry` は blake3 を 64 hex として検査するだけで、host / owner がその blob の Grant を持つかを assets に問わない（`rooms.ex` L310-330）。どこかの部屋の Manifest で見た hash を自室の scene に入れれば、その hash の PlaceRead を自分の部屋で永続的に得られる（「一度見た bytes は保存できる」ので機密漏洩としては小さいが、元の部屋の visibility 縮小を迂回する道になる）。(3) `World.HttpPop` は 60 秒の skew 内のリプレイを受理する（moduledoc で非保証と明記・TLS 前提なので -1 未満の扱いだが、seal / scene は状態変更なので nonce を入れたい）。
  > 改善方針: catalog に IP + user 軸の bucket。scene append 時に assets へ `HEAD /blobs/:blake3`（host sub の Grant 必須）を問い合わせる。PoP は `jti` 付き nonce を短命 ETS で 1 回限りにする。
  > 対象ファイル: `world-server/lib/world_web/plugs/rate_limit.ex`, `world-server/lib/world/rooms.ex`, `world-server/lib/world/http_pop.ex`

---

## 技術評価層 — 共通契約層

### protocol/

（当日の新規マイナスなし。`control_plane.rs` / `http_pop.rs` / AEAD v2 で純関数の共有契約がさらに厚くなった。）

### client/shared-sim

- **新しい XPBD 権威ホットパスが、プロジェクトの DOD / ゼロアロケーション規則に反する（新規）** `-1`
  > `XpbdWorld::commit_prev_from_curr` は `self.prev = self.curr.clone()`（`xpbd.rs` L44-46）で毎 tick SoA シート全体を確保し直す。host の `step_dt` はさらに `start = self.world.curr.clone()` と `snapshots.publish(start, self.world.curr.clone())` を行う（`host-server/src/sim.rs` L605, L629, L634）ので、**60Hz で全シートを 3 回 clone**。サブステップごとに `predict_integrate` が `Vec<u32>` を collect し（`xpbd.rs` L52-54）、`color_constraints` が `HashSet` と `Vec` を毎回作る（L94-120）。せっかく `constraint_scratch` / `sub_prev_scratch` を持ちながら、肝心の部分が漏れている。実装ルールの「フレーム間でアロケーションを回さない」に正面から反する。
  > 改善方針: `prev.clone_from(&curr)`（容量再利用）か、prev/curr を `std::mem::swap` してから curr を上書き。active rows は `EpochSheet` 側に再利用バッファを持たせ、彩色は世代番号付き `Vec<u32>` マーカーで HashSet を置き換える。`criterion` 相当（新規依存は要承認）が無ければ `#[test]` で確保回数を数えるカウンタアロケータを使う。
  > 対象ファイル: `client/shared-sim/src/xpbd.rs`（L44-120）, `client/host-server/src/sim.rs`（L593-636）

---

## 技術評価層 — ホストユーザー端末

### client/host-server

- **Rapier から自作 XPBD への置換で、物体同士の接触・Grab の回転追従が失われた（新規）** `-2`
  > x0093 で wire 権威を Epoch/XPBD に移した。設計（SoA シート・dirty/sleep ビット・`forbid(unsafe_code)`）は良いが、`sim.rs` の非保証は「Grab aim **rotation** follow; throw impulse; Rapier bit-exact parity; capsule Walk extrude; **LBVH body-body contacts on host**」（L11-13）で、`step_dt` は `xpbd::step(…, None, dt)` と bounds を渡さず「Body-body LBVH contacts intentionally off (x0093); ground plane only」（L606-614）。つまり**箱同士・メッシュ同士はすり抜ける**。Rapier 時代は collider で衝突していたので、ユーザーから見える物理は後退した。しかも `rapier3d = "0.22"` が workspace 依存に残っているのに、どのクレートも使っていない（`client/Cargo.toml` L42）。overview・`client.md`・`protocol.md` も Rapier と書いたまま（横断の docs 項）。
  > 改善方針: `FrameBridge` 経路では既に LBVH contact を通しているので、host でも `Some(bounds)` を渡して body-body を有効化し、回帰テスト（2 箱が重ならない）を足す。使わない `rapier3d` は workspace から外す（または「戻せる保険」なら理由をコメントで残す）。
  > 対象ファイル: `client/host-server/src/sim.rs`（L1-13, L593-636）, `client/Cargo.toml`（L42）

### client/app

- **`dash.rs` / `desktop.rs` の肥大がさらに進んだ（継続・悪化）** `-2`
  > 前回 -1 で指摘した 2 ファイルは、`system_ui/dash.rs` 2268 → **3604 行**、`surface/desktop.rs` 1815 → **3805 行**に倍増した。加えて `system_ui/mod.rs` 2352 行、`scene.rs` 2487 行。Worlds カタログ・Host 開始/handover/takeover・Inventory・auth handoff・hot switch が UI イベントハンドラ周辺に積み上がり、`desktop.rs` 3805 行に `#[test]` は 5 本しかない。system_ui が OpenXR / Room 固有知識を持たないという設計原則（Facet 分離）を守るためにも、要求組み立て（world_api / inventory / host_supervisor 呼び出し）を UI から分けるべき。
  > 改善方針: `surface/desktop/{session_flow,inventory_flow,hotswitch}.rs` に分割し、UI は「イベント → コマンド enum」だけを出し、コマンド実行器を別スレッド側のモジュールにする（テスト可能になる）。
  > 対象ファイル: `client/app/src/system_ui/dash.rs`, `client/app/src/surface/desktop.rs`, `client/app/src/system_ui/mod.rs`, `client/app/src/scene.rs`

- **GUI からホストを開始すると、約 12〜15 分で host が Room Token を再発行できなくなる（新規）** `-2`
  > HostSupervisor は UI ログインの access JWT を子プロセスの `ALCHEMY_AUTH_TOKEN` に渡すが、host はそれを `expires_in=0`（refresh 無し）として扱う。moduledoc 自身が「Room Token reissue fails after access JWT TTL (≈12–15 min). **Prefer env `AUTH_USER`(+password) for durable Host.**」と書く（`host_supervisor.rs` L26-33）。host は 15 秒ごとに host Token を取り直す設計なので（credential worker）、**通常のユーザー（環境変数を設定しない）が GUI で部屋を開くと、access JWT 失効後に hard expiry で配信が止まる**。回避策が「パスワードを環境変数に置け」なのも配布物として不適切。
  > 改善方針: client が refresh token を保持したまま、loopback IPC（既存の `host_ipc` + `ENV_TOKEN`）で host に「新しい access JWT」を push する（host は親からの更新を受ける）。refresh token 自体は子に渡さない現方針は維持できる。
  > 対象ファイル: `client/app/src/host_supervisor.rs`（L26-33）, `client/host-server/src/room_token/credential_chain.rs`, `client/host-server/src/host_ipc.rs`

- **GLB 画像の pixel 上限が「デコード後」に判定される（部分解消）** `-1`
  > x0085 の `AssetDecodeBudget`（入力 32MiB・展開 128MiB・頂点 50 万・index 150 万・pixel 4096²）は良い。ただし画像は `gltf::image::Data::from_source(...)` で**フルデコードしてから** `budget.add_image_pixels(width, height)` を呼ぶ（`mesh.rs` L229-232）。小さな PNG で巨大寸法を宣言する圧縮爆弾は、budget に弾かれる前に一度確保される。実際の上限は image 0.25 の既定 `Limits`（max_alloc 512MiB）頼みで、自前 budget（4096²×4 ≈ 64MiB）の 8 倍まで一時確保しうる。
  > 改善方針: `image::ImageReader::new(Cursor).with_guessed_format()?.into_dimensions()` でヘッダだけ読み、寸法を先に budget に掛けてからデコードする（`image` は既存依存）。
  > 対象ファイル: `client/app/src/mesh.rs`（L222-241）, `client/app/src/asset_decode_budget.rs`

### launcher/

（`launcher/` は git 管理から削除済み（`git ls-files launcher` = 0）で、HostSupervisor に移譲。評価対象外。ただし `windows-release.yml` L40 の rust-cache に `launcher -> target` が残る → 横断 docs 項に含める。）

---

## 技術評価層 — 転送・公開エッジ

### relay-server / edge / relay-agent

- **relay-agent の JWKS 取得が `curl` 子プロセスで、ロック取得に `expect` が残る（継続・軽微）** `-1`
  > x0083 で `key_pem` は消え、非ループバック待受は起動時と bind 後の 2 回で拒否される（`lib.rs` L241-277, L319）— 前回の主旨は閉じた。残るのは、JWKS を `Command::new("curl")`（L635-641）で取る外部コマンド依存と、`self.cache.write().expect("jwks lock")`（L381, L387）の poison 時 panic。互換経路として小さく保つならこのままでもよいが、host / client と同じ HTTP クライアント（ureq 等）に寄せれば Windows 配布でも curl 前提が消える（新規依存になる場合は要承認）。
  > 対象ファイル: `relay-agent/src/lib.rs`

---

## 横断評価層

### テスト戦略

- **ローカル CI の assets ステップが Elixir コンパイラのクラッシュで落ちる（新規・フレーク疑い）** `-1`
  > 当日 `./.scripts/ci.sh all` の `assets mix checks (Docker)` が、テスト以前の依存コンパイル（credo 257 ファイル）で `RuntimeError: found error while checking types for Credo.Check.ConfigComment.new/3 … UndefinedFunctionError … Credo.Check.ConfigComment.__info__/1`（Elixir 1.19.5 型検査）で停止した。auth / world は同じ credo で通っている。原因は assets の `deps` / `_build` 名前付きボリュームの共有（`docker-compose.yml` の `assets_build`）と、同時に起動していた開発用 assets コンテナとの競合が疑わしい。前回の `Database busy` / `rm_rf` フレーク（x0086 で reaper / GC を test で起動しない設定にして解消、`config/test.exs` L20-22）とは別原因。再実行結果は総合レポートに記録した。
  > 改善方針: CI 用の run では `-v assets_ci_build:/app/_build` のように別ボリュームを使い、開発コンテナと `_build` を共有しない。Elixir 1.19 の並列コンパイル型検査起因なら `mix deps.compile credo --force` を CI 側で 1 回挟む。
  > 対象ファイル: `.scripts/ci.sh`（L232-244）, `assets-server/docker-compose.yml`

### プロジェクト全体設計

- **アーキ文書・配布ワークフローに退役済み・置換済みの記述が残る（新規）** `-1`
  > (1) overview の SharedSimCrate は「Rapier3d 物理エンジン」（`overview.md` L155）、`client.md` L17 / L40 は「Rapier + user_root loco」「Rapier `from_entries`」、`protocol.md` L25 / L43 も Rapier。実装は x0093 で XPBD（`sim.rs` L1）。(2) README L15 / L17 に「launcher CI」「launcher discovery x0072 done」が残るが launcher は削除済み。(3) `windows-release.yml` L40 が `launcher -> target` をキャッシュ対象にしている。前回 x008a で docs 鮮度を閉じた直後に、また実装に文書が追いついていない。
  > 改善方針: スライスの完了条件に「overview / 該当 architecture md / README の該当語を grep して更新」を入れる（x008a と同じ手順の常設化）。
  > 対象ファイル: `.workspace/0_doc/architecture/overview.md`, `.workspace/0_doc/architecture/client.md`, `.workspace/0_doc/architecture/protocol.md`, `README.md`, `.github/workflows/windows-release.yml`
