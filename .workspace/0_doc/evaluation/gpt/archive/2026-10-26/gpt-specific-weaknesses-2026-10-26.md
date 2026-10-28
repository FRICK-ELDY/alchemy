# Alchemy 第2評価者 — 具体的な弱み（2026-10-26）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 軽微な改善余地 |
| -2 | 重要な機能・設計の欠如 |
| -3 | バグ・クラッシュ・性能劣化を起こしうる明確な欠陥 |
| -4 | 価値命題を損なう重大な欠如 |
| -5 | 根幹を揺るがす致命的欠陥 |

## 運営側コア

### auth-server

- **Refresh Token の同時ローテーションに排他がない** `-2`
  > `do_refresh` は transaction 内だが、対象 RefreshToken を `FOR UPDATE` せず、読取済み record を revoke して次 token を作る。同じ token の同時リクエストが双方で未失効判定を通り、同一 family に複数の有効 token を作る余地がある。取得時ロックまたは原子的な `revoked_at IS NULL` 更新が必要。
  > `ensure_refresh_token_usable(record)` → `rotate_refresh_token(record, ...)`（`auth-server/lib/auth/accounts.ex:307-327, 354-358`）。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

### assets-server

- **CAS hash を知る全認証ユーザーが他人の Blob を取得できる** `-4`
  > `show` は JWT を要求するだけで、`auth_sub` と `BlobRef.owner_sub` を比較しない。`fetch(blake3)` も hash だけで global row を返すため、所有者境界を保証できず、有料・非公開アセットの hash が Manifest 等から見えれば取得可能である。CAS object と owner reference を分離し、GET は所有権または明示 grant を検証すべき。
  > `Repo.get(BlobRef, blake3)`（`assets-server/lib/assets/blobs.ex:99-103`）。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **Quota と CAS/DB の並行整合性が保証されない** `-3`
  > ledger を row lock せず `used_bytes + size` 判定し、その後 `update_all(inc:)` するため、同一 owner の並行 upload は quota を超過できる。global BlobRef の同時 insert が競合した場合、CAS put 済みの orphan も残りうる。DB 一意制約だけでなく ledger の原子的条件更新、Blob object/reference 二段階化、失敗時補償が必要。
  > `Repo.get(QuotaLedger, owner_sub)` と後段 `update_all(inc:)`（`assets-server/lib/assets/blobs.ex:140-167`）。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

### world-server

- **任意ユーザーが任意 room の host role を発行できる** `-4`
  > 認証済みであれば token endpoint が room を `ensure` で自動作成し、request body の `"role": "host"` をそのまま署名する。room owner/ACL/招待能力がなく、guest client からでも host capability を取得できる。world-server が門番であるという現在の価値命題に反する。
  > `role = Map.get(params, "role", "guest")` と `Rooms.ensure(room_id)`（`world-server/lib/world_web/controllers/room_controller.ex:27-58`）。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

## 共通契約・ホスト

### protocol

- **room_id の検証契約が Rust と world-server で不一致** `-2`
  > Rust は ASCII printable と `/` 禁止だけで長さ上限なし、world は `[A-Za-z0-9_-]+` かつ 64 bytes、Room Token 内部検証はさらに文字集合を確認しない。ある層では有効、次の層では無効となる契約は運用事故になる。共通 schema test vector か単一 validator を生成すべき。
  > Rust `validate_room_id`（`protocol/src/lib.rs:74-88`）対 world registry（`world-server/lib/world/rooms.ex:75-91`）。
  > 対象ファイル: `protocol/src/lib.rs`

- **Protocol CI の protoc 対象が schema 全体ではない** `-1`
  > workflow とローカル CI は `render_frame.proto` と `input_events.proto` しか直接 compile せず、`scene_manifest.proto` と `presence.proto` の schema-only gate を欠く。Cargo build が補完する場合でも、契約検査としてリストが不完全。
  > `protoc ... render_frame.proto input_events.proto`（`.github/workflows/protocol-ci.yml:29-37`）。
  > 対象ファイル: `.github/workflows/protocol-ci.yml`

### client/host-server

- **認証されていない host_control で誰でも host を終了できる** `-5`
  > room の Zenoh key に publish できる主体は payload `shutdown` だけで `std::process::exit(0)` を発火できる。WAN router が transport identity/ACL を持たない現状と組み合わさると、部屋の可用性を外部入力一発で失う。スタブでも実際に subscribe・exit しているため評価対象外にはできない。
  > `if bytes.as_ref() == HOST_CONTROL_SHUTDOWN { std::process::exit(0); }`（`client/host-server/src/host_control.rs:20-27`）。
  > 対象ファイル: `client/host-server/src/host_control.rs`

- **有限値検査だけで magnitude・速度・入力頻度を制限しない** `-3`
  > `1e30` のような finite 値は通過し、move は速度へ直結、aim は Grab 対象の kinematic 座標へ直結する。署名済み guest が物理発散や任意遠距離移動を起こせる。各軸 clamp、ベクトル正規化、移動量上限、per-sub rate limit、異常 strike が必要。
  > `Non-guarantees: magnitude clamps, rate limits`（`client/host-server/src/finite.rs:1-14`）。
  > 対象ファイル: `client/host-server/src/finite.rs`

- **物理ループ内の同期 publish が 60Hz 保証を崩す** `-2`
  > Manifest/Presence/Frame の `.wait()` を物理 thread 内で実行する。Zenoh の backpressure や切断時に tick が停止し、catch-up 上限もない。simulation と network serialization/publish を bounded channel で分離し、tick drift metric を持つべき。
  > `frame_pub.put(...).wait()`（`client/host-server/src/main.rs:724-735`）。
  > 対象ファイル: `client/host-server/src/main.rs`

- **既定シード資格情報を実行コードが暗黙利用する** `-4`
  > `AUTH_USER` 未指定時に `alice / Password1` で login する。公開環境で env 設定を忘れた host が既知資格情報を使う fail-open 設計であり、README に同じ seed が公開されている。対話環境以外では token/env 必須で fail-closed にすべき。
  > `DEFAULT_AUTH_USER = "alice"`、`DEFAULT_AUTH_PASSWORD = "Password1"`（`client/host-server/src/room_token.rs:14-17, 170-183`）。
  > 対象ファイル: `client/host-server/src/room_token.rs`

## クライアント・公開エッジ

### client/app

- **Zenoh の Frame/Manifest/Presence を無認証・未検証で描画状態へ入れる** `-4`
  > client は publisher identity、host signature、room manifest signature を検証せず、Frame の NaN/Inf、色、entity 数、重複 revision も検査しない。悪意ある WAN peer が NaN matrix、巨大 repeated field、偽 Presence を注入できる。host が input を検証しても client endpoint は保護されない。
  > decode 後そのまま `map.insert(e.id, EntityPose { x: e.x as f32, ... })`（`client/app/src/main.rs:230-252`）。
  > 対象ファイル: `client/app/src/main.rs`

- **delta の順序・欠落・再同期契約がない** `-3`
  > `tick` と Manifest/Presence `revision` を読まず、古い packet、重複、欠落をすべて適用する。full snapshot/queryable もないため late join や packet loss 後に正しい state へ収束する保証がない。box を毎 frame upsert する回避策は delta 設計と帯域目標を崩す。
  > Frame subscriber は `frame.tick` を参照しない（`client/app/src/main.rs:230-255`）。
  > 対象ファイル: `client/app/src/main.rs`

- **remote player と box が同じ uniform buffer を上書きする** `-2`
  > box 9個用 buffer の末尾を remote root に再利用し、submit 前に queue write するため、box 7/8 等の draw も remote の最後の uniform を参照しうる。remote 専用 buffer/instance buffer が必要。
  > `let slot = (BOX_COUNT - 1 - i)...`（`client/app/src/scene.rs:614-633`）。
  > 対象ファイル: `client/app/src/scene.rs`

- **ログイン・登録・join を UI event thread で同期実行する** `-2`
  > Desktop の click handler 内で blocking `ureq`、world token、liveliness を順次実行する。遅い WAN や timeout 中に描画・入力が停止する。background task と UI state machine に分離すべき。
  > `handle_ui_action` 内の `auth::login` → `join_room_with_access`（`client/app/src/surface/desktop.rs:695-739`）。
  > 対象ファイル: `client/app/src/surface/desktop.rs`

- **整合性失敗時に無関係な fixture を黙って表示する** `-2`
  > download 失敗または Manifest hash 不一致時に Coin fixture へ fallback するため、ユーザーは「対象 asset が壊れた／改ざんされた」ことを視覚的に識別できず、entity と見た目が食い違う。開発専用 feature に限定し、本番は明示 error placeholder にすべき。
  > `Err(_) => { let fixture = load_fixture_bytes()?; ... fixture }`（`client/app/src/main.rs:579-587`）。
  > 対象ファイル: `client/app/src/main.rs`

### zenoh-router / edge

- **WAN 空間 transport が平文 TCP＋IP allowlist のみ** `-4`
  > `tcp/0.0.0.0:7447` は暗号化・相互認証・namespace ACL を持たず、Room Token は input payload だけを保護する。Frame、Presence、Manifest、host_control は偽装可能で盗聴も可能であり、「公開 WAN リレー」としては安全境界が成立していない。
  > `"tcp/0.0.0.0:7447"`（`zenoh-router/config.wan.json5:10-14`）。
  > 対象ファイル: `zenoh-router/config.wan.json5`

## 横断品質

### テスト戦略

- **ローカル assets CI が環境依存で失敗する** `-3`
  > 2026-10-26 に `bash .scripts/ci.sh assets` を実行すると、host の `assets-server/.env` にある S3 設定が Docker test へ残り、local GET を期待する test が 302 を受けて 7 tests 中 1 failure となった。`MIX_ENV=test` より後に runtime env が `cas_backend` を上書きするため、README の「同じゲート」は再現可能でない。
  > runtime が全 env で `ASSETS_CAS_BACKEND` を適用（`assets-server/config/runtime.exs:14-26`）。
  > 対象ファイル: `assets-server/config/runtime.exs`

- **サービス横断の自動 E2E が CI にない** `-2`
  > smoke script は多数あるが、auth login → Room Token → host gate → Zenoh → client state → assets redirect/hash を CI で通すジョブがない。現在の重大な認可・再同期問題は単体 test だけでは検出できない。
  > 対象ファイル: `.github/workflows/`

- **untrusted decoder/token/input の fuzz・property test がない** `-1`
  > Protobuf、JWT header/JWKS、GLB、room key、finite/magnitude は外部入力面だが、round-trip example test 中心である。cargo-fuzz/proptest/StreamData 等による adversarial test がない。
  > 対象ファイル: `protocol/src/lib.rs`

### 可観測性・エラー・保守性・セキュリティ

- **本番向け metrics/tracing/export が不足する** `-2`
  > Phoenix の標準 telemetry shell とログはあるが、host tick drift、Zenoh queue、JWT/JWKS refresh failure、CAS latency/quota reject、client frame age の統一 metric/exporter がない。分散構成で原因箇所を切り分けにくい。
  > 対象ファイル: `client/host-server/src/main.rs`

- **Mutex poison を正常な空状態として継続する** `-2`
  > Presence lock が poison すると default intent、revision 0、空 Presence に置換して simulation を続ける。内部 invariant 破壊を障害として停止・再起動せず、全ユーザー消失のような外形に変換する。recover 方針を明示すべき。
  > `.unwrap_or_else(|_| (... PresenceState { revision: 0, assignments: vec![] }, ...))`（`client/host-server/src/main.rs:600-623`）。
  > 対象ファイル: `client/host-server/src/main.rs`

- **entity ID と slot 上限が protocol/host/client に重複する** `-2`
  > user_root 3..6、box 10..18、slot 4 が複数 crate の定数として再定義され、schema には意味型がない。変更時に compile-time で不一致を検出できず、既に描画 buffer 設計にも影響している。protocol の scene definition か共有 simulation crate に寄せるべき。
  > host `USER_ROOT_SLOTS = 4`（`client/host-server/src/sim.rs:20-24`）と client（`client/app/src/scene.rs:24-34`）。
  > 対象ファイル: `client/app/src/scene.rs`

- **既知 HIGH/MEDIUM advisory を含む依存が CI を通過扱いになる** `-3`
  > assets CI 実行時、`hackney 1.25.0` に HIGH 1件、MEDIUM 2件等が表示されたが、workflow に `mix hex.audit` / `cargo audit` / deny gate がない。S3 HTTP 経路にある dependency なので放置できない。
  > `hackney 1.25.0` が lock されている（`assets-server/mix.lock:16`）。
  > 対象ファイル: `assets-server/mix.lock`

**弱み合計: -62**
