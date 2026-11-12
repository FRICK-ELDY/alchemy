# Alchemy — マイナス点詳細（第1評価者 Claude Opus 5.5 / 2026-11-05）

> 評価対象コミット: `c491016`（main・PR #142 マージ直後）。前回 `d2d9f44`（2026-10-30）から 114 コミット（x0066〜x007b の実装、inventory ladder x0054〜x0065 実装、x0053 hot switch 実装、4 階層ライセンス分離）。
> 全項目は当日コードを直接読んで確認した。第2評価者（`gpt/`）の文書は参照していない。
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
| 運営側コア | assets-server | -3 |
| 運営側コア | world-server | -8 |
| 共通契約 | protocol/ | -1 |
| ホスト端末 | client/host-server | -3 |
| ホスト端末 | client/app | -2 |
| ホスト端末 | launcher/ | 0 |
| 転送・エッジ | relay-server / edge / relay-agent | -1 |
| 横断 | テスト戦略 | -2 |
| 横断 | 可観測性 | 0 |
| 横断 | エラーハンドリング | 0 |
| 横断 | 変更容易性・保守性 | 0 |
| 横断 | 開発者体験（DX） | -1 |
| 横断 | セキュリティ・配布可能性 | 0 |
| 横断 | プロジェクト全体設計 | -2 |
| | **合計** | **-23** |

---

## 技術評価層 — 運営側コア

### auth-server

（当日の新規マイナスなし。前回唯一の `-1`「Keys の GenServer 直列化」は x0073 で `:persistent_term` 化され closed。）

### assets-server

- **BLAKE3 は常駐 Port 化されたが、全アップロードのハッシュが 1 本の GenServer に直列化される（部分解消）** `-1`
  > x0075 で `b3sum` CLI + 一時ファイルは消えた（`assets-server/lib/assets/blake3.ex` は `Worker.hash_hex/1` へ委譲するだけ）。ただし Worker は名前付き GenServer 1 つで Port 1 本を握り、`GenServer.call` で順番待ちになる。`max_upload_bytes` 近い大きな PUT が 1 件あると、その間の全ユーザーのアップロードが止まる。moduledoc は「parallel throughput」を非保証にしており誠実だが、Quota 予約 → 外部 put の並列設計（x003f）の上限がここで決まる。
  > 改善方針: Port を N 本のプール（`:poolboy` 相当は新規依存なので、`PartitionSupervisor` + `:erlang.phash2` で振り分ける）にするか、閾値以下のペイロードは呼び出し側プロセスで処理する。
  > 対象ファイル: `assets-server/lib/assets/blake3.ex`, `assets-server/lib/assets/blake3/worker.ex`

- **`CasObject.mime` が先着固定で、GET が `X-Content-Type-Options: nosniff` を付けない（新規）** `-1`
  > x006f で access の先着固定（policy squatting）は解消したが、`upsert_cas_object!/3` は「First writer wins for mime/size」（`blobs.ex` L310-319）のまま。誰でも有名な bytes を `Content-Type: text/html` で先に PUT でき、後続ユーザーの GET（local backend）は `put_resp_content_type(blob.mime)`（`blob_controller.ex` L38-41）でその mime のまま返る。Bearer 必須なので現状ブラウザから直接踏ませるのは難しいが、将来 CDN / 署名 URL で公開したときに型混同の足場になる。
  > 改善方針: mime は BlobRef（所有者ごと）に持たせるか、GET では `application/octet-stream` + `nosniff` を既定にし、表示用 mime はクライアントが Manifest 側から決める。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`（L310-319）, `assets-server/lib/assets_web/controllers/blob_controller.ex`（L36-41）

- **Origin Cert（VcsCert）の署名対象が `target_kind:target_id:subject_sub` だけで、内容ハッシュを含まない（新規）** `-1`
  > `Assets.Vcs.Cert.issue/3` は canonical message に commit / tag の **ID** と作者 `sub` しか入れず、`target_hash` は「その message の blake3」で署名対象外（`cert.ex` L14-16, L60-75）。overview の Origin Cert は「origin_id + 作者 sub + CommitHash + 親 ID」であり、証明書がバイト列に結び付かないと、DB 外（連合先・マーケット）で検証したときに「どの内容の作者か」を示せない。Tag は app 側で凍結（`vcs/ref.ex`）されているので現サーバー内では実害が小さいが、thin peel の時点で署名ペイロードの形を決めておかないと後から互換を壊す。
  > 改善方針: message に `commit.tree_blake3`（または blob blake3 の Merkle root）と `parent_commit_id` を入れ、`v1|` のようなバージョン接頭辞を付ける。
  > 対象ファイル: `assets-server/lib/assets/vcs/cert.ex`

### world-server

- **host Room Token が bearer のまま Zenoh 制御面（spawn / despawn）に載り、world の host 専用 API は PoP を見ない → 部屋メンバーによる host 乗っ取り連鎖（新規）** `-3`
  > x006d / x007b の client は `{"room_token": <host Room Token>, ...}` を JSON で組み、Room 鍵 AEAD で包んで `…/inventory/spawn|despawn` に put する（`client/app/src/inventory.rs` L171-206, L210-236）。AEAD の鍵は**部屋の全参加者が共有**する静的鍵なので、部屋メンバーなら誰でも復号できる。WAN ACL では guest は `inventory/*` を購読できないが、`host-sub-input`（`relay-server/config.wan.json5` L76-94）は `alchemy/dev/room/*/*/inventory/spawn` を**全部屋ワイルドカード**で許可しており、`alchemy-host` 証明書は「自分の部屋を 1 つ作る」だけで誰でも得られる（room cap 32）。LAN では ACL 自体が無い。
  > 盗んだ host Room Token（TTL 300s）で何ができるか: world の `POST …/host_keys` は `Token.verify` + `require_host_token` + `ensure_owner(room_id, claims["sub"])` しか見ず（`room_controller.ex` L416-422）、`cnf.spk` による所持証明は HTTP 側では一切検査しない（world 全体で `cnf` を読むのは発行時のみ・`token.ex` L21-29, L77-84）。攻撃者は自分の `host_pub` で HostAttestation を取り、`claim_host_generation` で generation を進め、canonical scene に対する ManifestSeal を正規に得て、より高い generation の Manifest を（host 証明書のワイルドカード put で）その部屋に流せる。client の `(generation, revision)` 単調ゲートは「新しい正規 host」として受理する。`POST/DELETE …/scene/entries` による scene 改ざんも同じ Token でできる。
  > 緩和要因: release の GUI は `fetch_guest_token` で常に `role: "guest"` を要求する（`client/app/src/world_token.rs` L121-124）ため、現在の配布ビルドでは spawn UI から host Token が流れる経路は実質ない（下の client/app 項）。つまり**潜在欠陥**だが、プロトコル（host-server が受理する payload 形）と world の API がこの形で確定しているので、owner の client に host Token を持たせた瞬間に顕在化する。
  > 改善方針: (1) 制御面の要求は Room Token ではなく **Intent と同じ session 鍵（Ed25519）署名**にし、host 側は bind 済み `sub` の role を見る（Token は bind 時に 1 回だけ）。(2) world の host 専用 API（host_keys / manifest_seal / scene entries）に `cnf.spk` の PoP（リクエスト署名 or DPoP 相当）を要求する。(3) WAN ACL の `inventory/*` 購読を host 証明書全体に開けない（下の提案「部屋束縛証明書」）。
  > 対象ファイル: `client/app/src/inventory.rs`（L171-236）, `world-server/lib/world_web/controllers/room_controller.ex`（L416-422, L541-547, L603-608）, `world-server/lib/world/token.ex`, `relay-server/config.wan.json5`（L76-94）, `client/host-server/src/inventory_spawn.rs`（L1-8, L30-35）

- **Room 鍵が部屋作成時に 1 度だけ生成され、公開範囲の変更・退室・フレンド解除でもローテーションされない（新規）** `-2`
  > `room_key` は `Rooms.create` の中で `generate_room_key_hex/0` により生成されるだけで（`rooms.ex` L628-642）、`set_visibility` を含むどの経路でも更新されない。Token 発行のたびに同じ鍵が配られる（`room_controller.ex` L202, L223）。WAN では guest 証明書（どこかの public 部屋に入れば得られる）で `*/*/manifest|frame|presence` をワイルドカード購読できる（`config.wan.json5` L143-160）ので、**一度でも部屋に入ったことのある人は、その部屋が後で `private` / `friends` に変わっても、フレンドを外されても、永久に平文を読める**。world の 7 段階公開範囲 ACL（x0034〜x0036）は「Token 発行時」の門番であり、x0069 以後は機密性の正本が Room 鍵に移ったので、鍵の寿命 = 公開範囲の実効寿命になってしまっている。x0069 スライスは「Room 鍵ローテ / 退室即時失効」を非保証に書いている（`x0069-room-key-aead.md` L15）が、world-server.md の保証表には `room_key` 自体が載っておらず、利用者からは見えない。
  > 改善方針: `room_key_epoch` を Room に持たせ、visibility 縮小・owner 操作（kick）・定期（例: 24h）で epoch を進める。AAD に epoch を入れ、host は新旧 2 鍵だけ受理（猶予 = Room Token TTL）。保証表に「epoch 更新前に配布済みの鍵は読める」を明記する。
  > 対象ファイル: `world-server/lib/world/rooms.ex`（L628-642）, `world-server/lib/world_web/controllers/room_controller.ex`（L196-238）, `.workspace/0_doc/architecture/world-server.md`, `.workspace/roadmap/slices/x0069-room-key-aead.md`（L15）

- **Room Token Bearer 系ルートに rate limit が無く、ManifestSeal の hash 計算は単一 Port GenServer に直列化される（新規）** `-1`
  > x0071 の `:rate_limited` は `:user_auth` スコープ（rooms / tokens / visibility）だけで、host Room Token / PlaceRead で叩く `host_keys` / `manifest_seal` / `scene/entries` / `place_read` / `place_hashes`（`router.ex` L24-38）は無制限。`manifest_seal` は canonical JSON を作って `World.ManifestSealHash.Worker`（名前付き GenServer 1 つ・Port 1 本、`manifest_seal_hash/worker.ex` L26-31）へ `GenServer.call` する。誰でも自分の部屋を作れば host Token を得られるので、1 人が自室の seal を連打するだけで**全部屋の seal 発行（＝全 host の配信再開）が待たされる**。
  > 改善方針: Room Token 系にも `sub` + `room_id` 軸の `World.RateLimit` を掛ける（x0071 の共有 `alchemy_rate_limit` をそのまま使える）。Port は assets 同様にプール化。
  > 対象ファイル: `world-server/lib/world_web/router.ex`（L24-38）, `world-server/lib/world/manifest_seal_hash/worker.ex`

- **`/health` が seal-hash Port の可用性を反映しない（継続・部分解消）** `-1`
  > x0075 で CLI 起動は常駐 Port になった（前回の「一時ファイル + 毎回起動」は解消）。しかし `HealthController.index/2` は固定 JSON を返すだけで、Port（`manifest-seal-hash --serve`）が落ちて再起動ループに入っていても 200 を返す。seal が出ないと全 host が配信停止（fail-closed として正しい）になるのに、ロードバランサ / 監視からは健康に見える。
  > 改善方針: `/health` を liveness のまま残し、`/ready` で Repo・Token.Keys・seal Port に ping する。
  > 対象ファイル: `world-server/lib/world_web/controllers/health_controller.ex`

- **共有 JWKS キャッシュが取得失敗時にクールダウンせず、GenServer 内で同期取得する（継続）** `-1`
  > `alchemy_auth_client` の `refresh_and_lookup_kid` は「成功後のみクールダウン」（`jwks.ex` L149-152）で、auth 停止中は未知 kid のたびに `Req.get(…, receive_timeout: 2_000)`（L187）を GenServer 内で同期実行する。world / assets の全リクエストがその GenServer に直列に並ぶ構図は前回と同じ。
  > 改善方針: host / client / relay-agent と同型の「失敗時も短い負のクールダウン」＋ Task での非同期取得。
  > 対象ファイル: `shared/alchemy_auth_client/lib/alchemy_auth_client/jwks.ex`（L143-193）

---

## 技術評価層 — 共通契約層

### protocol/

- ~~**参加者上限の天井 `USER_ROOT_SLOTS_MAX = 4` が protocol 定数のまま（継続）** `-1`~~ → **closed（x009b / P2-13）**
  > 帯を `USER_ROOT_ENTITY_FIRST=0x1000_0000` に移し、DoS 天井を 32 に拡大。部屋幅は Manifest 連続 entries。5 人超は pin 再 bump 不要。

---

## 技術評価層 — ホストユーザー端末

### client/host-server

- **`main()` が約 640 行の一枚岩のまま、むしろ伸びた（継続）** `-1`
  > x0076 で `room_token.rs`（2017 行）は `room_token/{verify,jwks,credential_chain,credential_worker,seal_cache,room_scene,timing}.rs` に分割され closed。しかし `main.rs` は 1430 行で、`fn main()` は L790 からファイル末尾近くまで（inventory spawn / despawn の適用、shared scene 差し替え、resume catch-up が追加された）。物理 tick を `fn tick(&mut HostLoop, now)` に切り出せばユニットテストできるという前回の改善方針はそのまま有効。
  > 対象ファイル: `client/host-server/src/main.rs`（L790-1430）

- **配信 tick ごとに pose 用 `Vec` を 2 本新規確保する（継続）** `-1`
  > `mesh_poses_from_sim(&physics)` と `Vec::with_capacity(...)` の `all_poses`（`main.rs` L1286-1301）を 30Hz で毎回作り、`FrameSample` として worker に move する。実装ルールの「フレーム間でアロケーションを回さない」に反する。worker からの返却チャネルでダブルバッファにすれば確保は起動時だけになる。
  > 対象ファイル: `client/host-server/src/main.rs`（L1286-1310）

- **bind の送信元分割キーが送信者の自己申告（Sample timestamp の uhlc ID）で、JWT 検証予算が最大 256 倍に広がった（部分解消）** `-1`
  > x0077 で「1 つの送信元が全体バケットを枯らして全員の入室を止める」は解消した（`intent_rate.rs` L6-12）。ただし source キーは `sample.timestamp()` 由来（`main.rs` L551-557）で、ID は publisher が自由に変えられる。`MAX_BIND_SOURCES = 256` と idle GC で RAM は有界だが、(a) 256 個の偽 ID を「成功消費」で温め続ければ新規送信元の bind を拒否できる（moduledoc L20-21 が自認）、(b) 旧来の host 全体 10/s が消えたので、JWT 検証 CPU の上限は 256 × 10/s に広がった。x0069 以後は Intent も AEAD なので攻撃者は「その部屋の鍵保持者」に限られ、実害は縮小している。
  > 改善方針: bind 前に AEAD 開封済み・Ed25519 検証済みである点を生かし、`session_pub` の初回出現に小さな PoW / 共有全体上限（成功 bind で返却）を併用する。
  > 対象ファイル: `client/host-server/src/intent_rate.rs`（L1-50）, `client/host-server/src/main.rs`（L545-560）

### client/app

- **Inventory take-out / despawn UI は host Room Token 前提だが、release の client は guest Token しか取得しない（新規）** `-1`
  > `desktop.rs` L1476 / L1654 は `room_token_role_unverified(&room_token) != Some("host")` なら「join a room as host first」で止める。一方 `fetch_guest_token` は `role: "guest"` 固定（`world_token.rs` L121-124）で、host Token を得る経路は debug の `ALCHEMY_ROOM_TOKEN` だけ（L97-112 は `cfg!(debug_assertions)`）。README / x006d / x006e は take-out・put-back UI を **done** とするが、配布ビルドでは UI からは到達しないように読める。上の world 項の「host Token を Zenoh に流す設計」と合わせて、owner 用の権限経路を設計し直す必要がある。
  > 改善方針: owner の client は guest Token のまま、spawn 要求を session 鍵で署名し、host が `sub == room.owner_sub`（world から scene と一緒に受け取る）で判定する。
  > 対象ファイル: `client/app/src/surface/desktop.rs`（L1460-1500, L1645-1680）, `client/app/src/world_token.rs`（L90-156）, `.workspace/roadmap/slices/x006d-host-take-out-control.md`

- **`main.rs` の肥大は解消したが、`system_ui/dash.rs`（2150 行）と `surface/desktop.rs`（1754 行）が新しい一枚岩になった（新規）** `-1`
  > x0076 で `main.rs` は 1899 → 274 行になり、`bus/{manifest,frame,presence}.rs` に分かれた（closed）。代わりに Inventory / put-back / hot switch / オンボーディングの UI ロジックが `dash.rs` と `desktop.rs` に積み上がった。`desktop.rs` には HTTP 呼び出し（take_out）・Zenoh put・auth ロック取得が UI イベントハンドラ内で直接並んでいる（L1460-1500）。system_ui が OpenXR / Room 固有知識を持たないという前回の長所を守るためにも、Inventory の「要求組み立て」は `inventory` モジュール側へ寄せるべき。
  > 対象ファイル: `client/app/src/system_ui/dash.rs`, `client/app/src/surface/desktop.rs`

### launcher/

（当日の新規マイナスなし。前回の「host 再起動後の IPC stale」は x0072 discovery で closed。）

---

## 技術評価層 — 転送・公開エッジ

### relay-server / edge / relay-agent

- **Relay Agent（互換フォールバック）は今も鍵をサーバー側で生成し `key_pem` を平文 HTTP で返す（部分解消）** `-1`
  > x0066 で正経路は「client が CSR を作り world が署名」（`world-server/lib/world/mtls_issuer.ex` L28-33「Does not return the client private key」）になり、P0-4 の主旨は満たした。しかし relay-agent 自体は `openssl req -newkey rsa:2048 -nodes` で鍵を作り（`relay-agent/src/lib.rs` L507-521）、`AllocateResponse.key_pem`（L80-89, L700-708）で返し続ける。非 loopback 待受でも warn だけで起動する（L231-239）。加えて JWKS 取得は `curl` 子プロセス（L583-594）、`jwks_age()` は `expect("jwks lock")`（L344-346）で poison 時に panic、10 秒 read timeout × `MAX_CONNECTIONS = 32` で slowloris 的に `/health` も含め埋まる。moduledoc は「CSR-only allocate (P0-4)」を非保証に書いており誠実だが、互換経路として残す限り同じ重さの門番である。
  > 改善方針: `/v1/allocate` も `csr_pem` 必須にして `key_pem` を廃止（world と同じ `-subj` 強制）。非 loopback は TLS 終端の背後でなければ起動拒否。それができないなら relay-agent を退役させる。
  > 対象ファイル: `relay-agent/src/lib.rs`（L1-8, L80-89, L231-239, L344-346, L483-566, L583-594）

---

## 横断評価層

### テスト戦略

- **assets の `mix test` が既定並列でフレークする（新規）** `-1`
  > 当日 `./.scripts/ci.sh all` の初回で `assets mix checks (Docker)` が **168 tests, 54 failures**（`Exqlite.Error Database busy` と `File.Error could not remove … "priv/cas_test": file already exists`）。同じコンテナで `mix test --max-cases 1` は 0 failures、既定設定の再実行も 0 failures。テスト環境でも `ReservedReaper` / `OrphanGc` が起動し（`application.ex` L25-26、`config/test.exs` L21-25）、`priv/cas_test` を共有する rm_rf 型の setup と SQLite の単一ライタが競合していると推定する（Windows Docker Desktop の bind mount で顕在化しやすい）。GitHub Actions（ubuntu）で緑でも、「ローカル CI が緑 = PR が緑」の前提を揺らす。
  > 改善方針: test では reaper / GC を `start: false`（明示起動テストだけ `start_supervised`）にし、CAS パスをテストごとの `tmp_dir`（ExUnit `@tag :tmp_dir`）にする。
  > 対象ファイル: `assets-server/lib/assets/application.ex`（L15-33）, `assets-server/config/test.exs`, `assets-server/test/**`

- ~~**旧 smoke スクリプトが現行ログ文言と乖離して腐っている（継続）** `-1`~~ **closed（x0089）**
  > banner-grep smoke（handthrough / noclip / lan-two-client / leave-liveliness / presence-slots）を削除。機械検証は `make ci-e2e`。P2-10（client.md / wan コメント等）は **x008a** で closed。

### 開発者体験（DX）

- **`make ci` が共有 Elixir パッケージ 3 つと fuzz を覆わない（継続・部分解消）** `-1`
  > relay-agent は `ci.sh relay` / `all` に入った（closed）。`shared/alchemy_*` は **x0089** で `ci.sh shared` / `all` に追加（closed）。残りは fuzz（`make ci-fuzz` は別入口のまま）。
  > 対象ファイル: `.scripts/ci.sh`, `.github/workflows/alchemy-rate-limit-ci.yml`

### プロジェクト全体設計

- ~~**保証表・設定コメントの鮮度に穴がある（継続・部分解消）** `-1`~~ **closed（x008a）**
  > `client.md` に `{wire_cap}`、`config.wan.json5` / `relay-server-wan.md` の第2壁を Room 鍵 AEAD、`world-server.md` に x0066 / x0069、`protocol.md` / `relay-server.md` キー式に inventory を追記。

- ~~**`vision.md` が見出しのみ（継続）** `-1`~~ **closed（x008a）**
  > 最終目標の短文を記載。overview / README / roadmap へ誘導。
