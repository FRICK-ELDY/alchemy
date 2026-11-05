# Alchemy — プラス点詳細（第1評価者 Claude Opus 5.5 / 2026-11-05）

> 評価対象コミット: `c491016`（main・PR #142 マージ直後）。前回 `d2d9f44`（2026-10-30）から 114 コミット。
> 「継続」は前回の長所を当日コードで再確認したもの、「新規」は x0066〜x007b / x0053 / x0054〜x0065 / ライセンス分離で新たに入ったもの、「更新」は前回の長所が実装変更で形を変えたもの。
> 第2評価者（`gpt/`）の文書は参照していない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている。問題はないが特筆するほどではない |
| +2 | 業界の一般的なベストプラクティスに沿った、良い設計判断 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る実装 |
| +4 | プロダクションレベルのゲームエンジン・OSSと比較しても遜色ない実装 |
| +5 | このクラスの個人プロジェクトでは見たことがないレベルの卓越した実装 |

| ブロック | 観点 | 小計 |
|:---|:---|---:|
| 運営側コア | auth-server | +25 |
| 運営側コア | assets-server | +29 |
| 運営側コア | world-server | +34 |
| 共通契約 | protocol/ | +34 |
| ホスト端末 | client/host-server | +43 |
| ホスト端末 | client/app | +43 |
| ホスト端末 | launcher/ | +7 |
| 転送・エッジ | relay-server / edge / relay-agent | +21 |
| 横断 | テスト戦略 | +26 |
| 横断 | 可観測性 | +9 |
| 横断 | エラーハンドリング | +6 |
| 横断 | 変更容易性・保守性 | +13 |
| 横断 | 開発者体験（DX） | +9 |
| 横断 | セキュリティ・配布可能性 | +13 |
| 横断 | プロジェクト全体設計 | +19 |
| | **合計** | **+331** |

---

## 技術評価層 — 運営側コア

### auth-server（アイデンティティ SSoT）

- **Refresh Token のファミリー回転・再利用検知・行ロック付き回転（継続）** `+4`
  > 前回確認した設計に変化なし。再利用検知でファミリー全体を失効させる構成は OAuth 2.1 BCP 相当。
  > 対象ファイル: `auth-server/lib/auth/token/`

- **署名鍵を `:persistent_term` に置き、未準備時は fail-closed で raise する（新規・x0073）** `+1`
  > `signer/0` / `jwks/0` / `signer_for_kid/1` は `:persistent_term.get(@pt_key, :missing)` を読むだけになり（`keys.ex` L9-13, L62, L83）、GenServer 直列化が消えた。「空の JWKS を返さず raise」と moduledoc で明示し、`await_ready/1` を起動同期に使うのも正しい。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`

- **RS256 + `kid` + ローテーション用検証鍵（継続）** `+3`
- **多軸レート制限（IP × 識別子 × トークンファミリー）と初回超過のみ telemetry（継続）** `+3`
- **Social mutate を `:authenticated` 後段で user + IP 両軸の rate limit に載せた（継続・x0043）** `+1`
- **Tunnel 背後のクライアント IP 解決を `shared/alchemy_client_ip` で共有（継続）** `+2`
- **Follow / Friendship の DB 制約付きソーシャルグラフ（継続）** `+2`
- **アカウント列挙・タイミング攻撃への配慮（継続）** `+2`
- **メール確認・パスワードリセットのトランザクション設計（継続）** `+2`
- **認証 Plug の失敗分類と情報を漏らさない構造化ログ、`access_jti` をログ metadata に（継続・拡張 x007a）** `+2`
  > `config.exs` L95-96 の Logger metadata が `[:request_id, :access_jti]` になり、入室フローの横断追跡の起点を持った。
  > 対象ファイル: `auth-server/config/config.exs`
- **S2S 内部トークンの prod fail-closed と定数時間比較（継続）** `+1`
- **公開 / レート制限 / 認証必須 / 内部のパイプライン分離（継続）** `+1`
- **auth の loopback metrics（x007a）** `+1`
  > world / relay と同型の依存なし Prometheus テキスト。公開ポートに載せない方針が trust-boundary と一致。

### assets-server（CAS・Quota・門番）

- **CAS の物理（`CasObject`）から access を剥がし、権利を `BlobGrant` / PlaceRead だけに置いた（更新・x006f）** `+4`
  > 前回まとめ P0-5 の「access 先着固定（policy squatting）」を、カラムごと消すことで解いた。`CasObject` は bytes メタのみ（`blobs.ex` L12-16）、同一 blake3 の後続 PUT は自分の `BlobRef` + owned Grant を足すだけ（L41-47, L94-116）。GET は「`auth_sub` の Grant」か「PlaceRead が blake3 をカバー」の 2 通りしか無い（L459-470）。物理・所有・権利の 3 分離を「権利側だけが capability を持つ」まで徹底したのは、IPFS / git-lfs 系の素朴な CAS より一段上。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`, `assets-server/lib/assets/blobs/blob_grant.ex`

- **GET を常時 Grant / PlaceRead で門番する（案 B・新規・x006c）** `+3`
  > 「認証済みなら読める」既定を廃止し、場（world seal）に紐づく PlaceRead JWT（`typ=place_read` / `aud=alchemy-assets-place-read` / `sub` 一致 / seal 時固定の hash 列のみ・`extra_blake3` 無効）で読ませる（`assets-server/lib/assets/place_read.ex` L1-9）。「見る権利」を部屋と seal に束縛した設計は overview の verifier 目標（見る権利 / 着る権利の分離）への正しい前段。
  > 対象ファイル: `assets-server/lib/assets/place_read.ex`, `assets-server/lib/assets/place_hash_cache.ex`

- **reserved reaper + orphan CAS GC を「quarantine → 再確認 → discard / restore」で TOCTOU 安全に（新規・x0074）** `+3`
  > 補償パスでは決して `CAS.delete` しない、という前回の長所を保ったまま、残骸を別プロセスで回収する。削除前に一度隔離し、CasObject / 予約を再確認してから捨てる（`orphan_gc.ex` L1-10）。予約解放は「行を消した TX だけが `reserved_bytes` を減らす」（`blobs.ex` L249-273）で二重減算を防ぐ。並行 PUT とのレースを正面から扱っており、S3 系 GC の実装として水準以上。
  > 対象ファイル: `assets-server/lib/assets/cas/orphan_gc.ex`, `assets-server/lib/assets/quota/reserved_reaper.ex`, `assets-server/lib/assets/blobs.ex`

- **Quota 予約 → TX 外 CAS put → 確定、補償は予約解放のみ（継続・x003f）** `+3`
- **検疫を `Assets.Quarantine.Worker` Behaviour + 安定公開コード `public_reason/1` に分離（新規・ライセンス境界）** `+2`
  > OSS にはヘッダ検査の既定 Worker だけを置き、pgvector 等の Proprietary 実装は設定注入（`quarantine.ex` L1-11）。API は `inspect/1` を返さず、未知理由は `"rejected"` に潰す。ライセンス境界と情報漏えい防止を同じ継ぎ目で解いている。
  > 対象ファイル: `assets-server/lib/assets/quarantine.ex`, `assets-server/lib/assets/quarantine/*.ex`

- **BLAKE3 を NIF ではなく常駐 Port（`{:packet, 4}`）で計算（新規・x0075）** `+2`
  > 一時ファイルと毎回のプロセス起動を消しつつ、NIF の「BEAM ごと落ちる」リスクを避けた。Port の stale EXIT / shutdown を締めるレビュー修正（`baceeb6`）まで入っている。
  > 対象ファイル: `assets-server/lib/assets/blake3/worker.ex`, `protocol/src/port_framing.rs`

- **メタと不変バイナリの分離 + BLAKE3 CAS（継続）** `+2`
- **CAS バックエンドの behaviour 抽象と起動時の必須 env 検査（継続）** `+2`
- **s3 時の GET を presigned URL への 302 にする（継続）** `+2`
- **条件付き UPDATE による Quota の原子更新（継続）** `+2`
- **Drive / VCS（branch 可変・tag 凍結）の薄皮を Grant 検証付きで実装（新規・x0054〜x0065）** `+1`
  > `vcs/ref.ex` で tag の `commit_id` 更新を拒否、Drive の take_out は owner + Grant 検証済み blake3 のみ返す。署名対象の弱さはマイナス側に記載。
- **`Plug.Parsers` 撤去による生 body 受信（継続）** `+1`
- **テスト環境で CAS 系 env を無視する隔離（継続）** `+1`
- **local CAS の原子的書き込み（tmp → rename）（継続）** `+1`

### world-server（案内所・門番・監査）

- **ManifestSeal を DB の scene + 検証済み attestation から world 自身が計算する（継続・x0039）** `+4`
- **CSR から短命 mTLS を署名し、CN をサーバーが強制、NotAfter = TTL、CA 一時ファイルを必ず消す（新規・x0066 / x0068）** `+3`
  > `World.MtlsIssuer.issue/3` は client の秘密鍵を一切受け取らず、CSR の Subject を無視して `-subj /CN=alchemy-host|guest` を強制し（`mtls_issuer.ex` L222-244）、`-not_before now-60s` / `-not_after now+ttl` で証明書寿命を Token の 300s に揃える（L215-221）。inline CA は 0600 で書いて `after` で必ず消し、chmod 失敗も fail-closed（L140-171, L264-273）。CA 設定済みで署名失敗なら Token ごと 5xx（`room_controller.ex` L376-401）。前回 P0 の「24h 証明書」「平文鍵配布」をまとめて正面から解いた。
  > 対象ファイル: `world-server/lib/world/mtls_issuer.ex`, `world-server/lib/world_web/controllers/room_controller.ex`

- **PlaceRead の `exp` を検証済み seal の絶対 `exp` に一致させる（新規・x006b）** `+3`
  > 「PlaceRead が seal より長生きしない」を exp の一致で機械的に保証（`world/place_read.ex` L1-10）。再起動後は re-seal 必須・退出即時 revoke は非保証と明記。capability の寿命設計として筋が良い。
  > 対象ファイル: `world-server/lib/world/place_read.ex`, `world-server/lib/world/place_hashes.ex`

- **HostAttestation / ManifestSeal の発行（typ・aud 分離の RS256 JWT）と host_generation（継続）** `+3`
- **Room 作成時に 32B の Room 鍵を CSPRNG で発行し、Allocation JWT / relay には載せない（新規・x0069）** `+2`
  > `:crypto.strong_rand_bytes(32)`（`rooms.ex` L639-642）、Token JSON では URL-safe base64、Relay 側には配らない（x0069 実装メモ）。ローテーションが無い点はマイナス側に記載。
- **rate limit を `:user_auth` 後段の Plug に分け、IP / user 両軸 + owner ごとの Room 上限（新規・x0071）** `+2`
  > `WorldWeb.Plugs.RateLimit`（`rate_limit.ex` L17-38）は action ごとに軸を決め、`retry-after` を返す。`ensure_under_owner_cap/1`（`rooms.ex` L579-604）は設定値不正を `storage_error` に倒す fail-closed。
- **Room Token に `cnf.spk`（RFC 7800 相当）を埋め込む（継続）** `+2`
- **Room の永続化（Ash + SQLite）とストレージ障害の 503 分離（継続）** `+2`
- **`host` ロールは owner のみ（継続）** `+2`
- **公開範囲の Token 発行時 ACL（auth への S2S・fail-closed）（継続）** `+2`
- **Room Token の claim 設計 + `jti` を Logger metadata / 発行ログに（継続・拡張 x007a）** `+2`
- **auth JWT 検証と `sub` の転写（継続）** `+2`
- **Verifier を `AuditEngine` Behaviour の注入点だけに留め、「呼び出し側は未配線」と明記（新規）** `+1`
  > `World.Verifier` は `:world, :audit_engine`（既定 `NoopAuditEngine`）に委譲するだけで、Spot-Check の閾値を OSS に置かない（`verifier.ex`, `verifier/audit_engine.ex`）。未配線を保証表で隠さない点も良い。
- **Zenohex を `ZenohexAdapter` 1 モジュールに隔離（新規・x006a）** `+1`
  > `%Zenohex.Sample{}` を外に出さない・再購読しない・GenServer.call 直列化を非保証に明記（`zenohex_adapter.ex` L1-22）。NIF 依存の影響範囲を 1 ファイルに閉じた。
- **`wire_cap` の発行と Allocation Token（`aud=alchemy-relay`・`relay_id` 束縛）の分離（継続）** `+1`
- **`lan` 公開範囲の fail-closed 判定（継続）** `+1`
- **匿名トークン既定 OFF（継続）** `+1`

---

## 技術評価層 — 共通契約層

### protocol/（ワイヤ契約 SSoT）

- **Room 鍵 AEAD（XChaCha20Poly1305・192bit 乱数 nonce・AAD = `room_id|kind`）を protocol に集約（新規・x0069）** `+4`
  > `wire_aead.rs` は `seal/open` と `AeadKind`（frame / snapshot / manifest / presence / input / inventory spawn・despawn を区別）だけを持ち、AAD に `alchemy/aead/v1|{room_id}|{kind}` を入れて別部屋・別チャネルへの暗号文差し替えを拒否する（L22-30, L92-140）。送信者が複数いるので nonce 衝突を構造的に避けられる XChaCha を選んだ判断、「復号失敗は protobuf / 署名検証の前に discard」の順序、上限は plaintext 上限 + 40 という整合まで含め、前回 P0 の「部屋隔離が transport ACL に依存」を**アプリ層暗号で根本から**解いた。Matrix / MLS ほどではないが、VRChat / Resonite 系の個人実装では見ない水準。
  > 対象ファイル: `protocol/src/wire_aead.rs`, `protocol/src/lib.rs`（L48-54）

- **decode 前のバイト上限 + decode 後の件数・文字列上限（継続）** `+3`
- **`(host_generation, revision)` 単調ゲートを純関数で共有（継続）** `+3`
- **チャネル責務の分離（Manifest / frame / presence / input / liveliness / snapshot）（継続）** `+3`
- **Ed25519 ワイヤ署名の `wire_crypto` 集約（継続）** `+3`
- **配送クラス契約（peer-delivery.md）と `decide_frame_tick`（継続）** `+3`
- **Port 用 length-prefix framing と `blake3-hash` / `manifest-seal-hash --serve` を protocol 側に（更新・x0075）** `+2`
  > Elixir 2 アプリが同じ Rust エンコード経路を常駐プロセスで使う。`port_framing.rs` にテスト 5 件。
- **`PROTOCOL_PIN` をワイヤに載せ不一致を検出（継続・`0.18.0-x0069`）** `+2`
- **TRS と正規化ヘルパ（継続）** `+2`
- **role → capability の写像（`sim_capability`）（継続）** `+2`
- **client 外の独立クレート + protoc スキーマ検査（継続）** `+2`
- **キー式ヘルパ・`validate_wire_cap`・inventory spawn / despawn キー（継続・拡張）** `+2`
- **`user_root_slots_from_entries` で部屋容量を Manifest から導出（継続）** `+1`
- **`validate_room_id` を world と同じ規則に統一（継続）** `+1`
- **DemoScene 共有テストベクタ（`protocol/testdata/demo_scene_seal.json`）を正本に（新規・x0079）** `+1`
  > Elixir `World.DemoSceneVector` と Rust `demo_scene_vector.rs` が同じ JSON の期待 hash を照合する。前回の「keep in sync manually」を機械検査に置き換えた。

---

## 技術評価層 — ホストユーザー端末

### client/host-server（Untrusted 計算ホスト）

- **credential worker と「fail-closed = 失効で配信停止、失敗で殺さない」（継続）** `+4`
- **Zenoh put を network worker に分離し、物理はジョブ投入のみ（継続）** `+3`
- **bind で `cnf.spk == session_pub` を照合（継続）** `+3`
- **Intent rate limit の「どこで数えるか」の設計（継続・x0077 で送信元分割）** `+3`
  > 「両方のバケットにトークンがあるときだけ両方消費」「deny ではマップを増やさない」「idle は成功消費基準」と、偽 ID ローテーションに対する RAM 上限まで moduledoc で保証している（`intent_rate.rs` L1-22）。キーが自己申告である点はマイナス側。
- **Intent セッション表（継続）** `+3`
- **world 定義の Scene から Rapier を構築（継続）** `+3`
- **Presence 解放の三重化と、liveliness を検証済み jti に紐付け（継続）** `+3`
- **`room_token.rs`（2017 行）を保証単位 7 ファイルに分割（新規・x0076）** `+2`
  > `verify` / `jwks` / `credential_chain` / `credential_worker` / `seal_cache` / `room_scene` / `timing`。前回指摘した「JWKS を直すつもりで worker のバックオフに触る」リスクが構造的に消えた。
  > 対象ファイル: `client/host-server/src/room_token/`
- **inventory spawn / despawn を物理 tick 外の worker で処理し、world 追記成功後に scene + 再 seal を必ず適用（新規・x006d / x007b）** `+2`
  > HTTP は spawn worker、物理は ready な `SceneEntry` を 1 tick 1 件だけ受け取る（`inventory_spawn.rs` L1-8, L69-78）。Rapier が拒否しても canonical scene と seal は先に進め、「Manifest と PlaceRead の整合を物理より優先する」と明記。shared_scene の revision 単調適用（x007b review）も入っている。Token の運び方はマイナス側。
- **JWKS の SWR・未知 kid クールダウン・hard stale（継続）** `+3`
- **move の単位球クランプと Grab 開始距離 + 保持中クランプ（継続）** `+3`
- **snapshot queryable による純デルタ化と tick 規則（継続）** `+2`
- **内部物理 60Hz と配信 30Hz の分離 + 固定ステップ（継続）** `+2`
- **入力ゲートの順序と拒否理由ごとのカウンタ（継続・AEAD 開封を最前段に追加）** `+2`
- **Room Token 検証の厳格さ（継続）** `+2`
- **Rapier の SoA 構成と Grab の no-steal（継続）** `+2`
- **Mutex の poison を致命的エラーとして扱う（継続）** `+1`

### client/app（体感・描画・UI エンドポイント）

- **Manifest / Presence の検証失敗は discard、退室は明示 Leave のみ（継続）** `+4`
- **XR 描画を wgpu SceneGpu に統合（継続）** `+4`
- **信頼ゲートを `client/trust` クレートに切り出し、GUI と headless probe が同じ関数を通る（新規・x0070）** `+3`
  > `bus/manifest.rs` / `bus/presence.rs` / `main.rs` は `alchemy_client_trust::{process_manifest_bytes, gate_presence_pin, ManifestSealVerifier, …}` を使い、`trust-probe` も同じ関数で偽 seal / 別鍵を流す（`trust-probe.rs` L1-18）。「E2E が本物の client 経路を通らない」という前回 P1-2 を、ゲートの所在を変えることで解いた。テスト用の別実装を作らない判断が正しい。
  > 対象ファイル: `client/trust/src/`, `client/app/src/bus/`
- **gen/rev ゲートを両ロック保持下で判定・適用（継続）** `+3`
- **Frame / Presence の署名検証と seal + attestation の信頼チェーン（継続）** `+3`
- **system_ui が OpenXR / winit / Room を知らない（継続）** `+3`
- **OpenXR の実機コントローラ描画（継続）** `+3`
- **`main.rs` 1899 行を `bus/{manifest,frame,presence}.rs` 等へ分割し 274 行に（新規・x0076）** `+2`
- **Desktop / XR hot switch で Dash の論理状態を SharedState に共有し、chrome / form を分けて CAS（新規・x0053）** `+2`
  > `surface/hotswitch.rs`。F8 で表示面を切り替えても UI 状態が失われない。
- **assets 失敗は placeholder、release で fixture に落ちない、HTTP timeout（継続）** `+2`
- **client world JWKS の soft / hard + 未知 kid クールダウン（継続）** `+2`
- **access / Room Token の 80% 自動更新と liveliness の付け替え（継続）** `+2`
- **surface の自動選択とフォールバック（継続）** `+2`
- **デコード前の BLAKE3 照合と 302 追従（継続）** `+2`
- **利用規約同意をチェックボックスで取り、未同意なら送信しない（新規・前回 -1 の解消）** `+1`
  > `auth.rs` L140-155 は `tos_agreed` を引数で受け `false` なら bail、`desktop.rs` L1288-1289 が UI 状態から渡す。
- **mTLS 鍵ファイルを作成直後に DACL / 0600 で絞り、管理下の古い鍵だけを掃除（新規・x0078）** `+1`
  > 空ファイル作成 → 権限制限 → 書き込みの順（`zenoh-connect/src/allocate.rs` L280-345）。Windows では `icacls /inheritance:r /grant:r` を実際に検証するテストがある（L755-790）。
- **ログイン・入室の HTTP をワーカースレッドで実行（継続）** `+1`
- **新規ワールド作成とアクセスレベル選択（継続）** `+1`
- **Leave → liveliness undeclare → session close の順序設計（継続）** `+1`
- **サブリレー化しない純粋エンドポイント（継続）** `+1`

### launcher/（プロセスマネージャ）

- **再起動ポリシーを純関数 `decide_host_exit` に切り出し CI に載せた（継続）** `+2`
- **ready ファイルでの IPC アドレス通知（TOCTOU なし）（継続）** `+2`
- **discovery（`127.0.0.1:0`・セッション固定 token）で host 再起動後の IPC を再解決し、crash 中は LOOKUP を ERR にする（新規・x0072）** `+2`
  > client は SHUTDOWN の直前に LOOKUP し、成功時のみ送る。偽 OK / stale fallback を閉じたレビュー修正（`c743bbe`）まで入っている（`launcher.md` L16-27, `launcher/src/discovery.rs`）。前回 -1 を closed。
- **launcher の独立 lock も cargo deny でゲート（新規・x0073）** `+1`

---

## 技術評価層 — 転送・公開エッジ

### relay-server / edge / relay-agent

- **zenohd ACL を「キー族 × 非対称 put + `declare_subscriber` egress」に組み直し、trace の根拠をコメントに残した（新規・x0067）** `+3`
  > guest は sim キーを **put ingress できない**（購読と put egress のみ）、host→guest / guest→host の両方向で interest egress を明示許可（`config.wan.json5` L8-13, L50-58, L110-190）。zenohd 1.10.1 の挙動を WAN 実機で trace して「なぜこの egress が要るか」を書いているので、次の人が「余計に見える allow」を消して壊す事故を防げる。Zenoh ACL をここまで詰めた公開設定は珍しい。
  > 対象ファイル: `relay-server/config.wan.json5`

- **relay-agent を「NotAfter = 300s・hard JWKS・body / 接続 / in-flight 上限・jti dedupe・未知 kid の per-kid クールダウン（上限付き）」に作り直した（新規・x0068）** `+3`
  > `-not_after` で秒単位の寿命（`lib.rs` L493-556）、`JWKS_HARD_TTL` 600s で stale 鍵の無期限受理を停止（L349-363, L423-434）、`MAX_BODY_BYTES` 16KiB を Content-Length の時点で 413（L605-640）、`MAX_IN_FLIGHT` 8 / `MAX_CONNECTIONS` 32 を RAII ガードで（L156-179, L293-305）、openssl 前に jti を予約し失敗時に解放（L677-690）。前回 P0 の 3 項目を一括で閉じ、NotAfter を実際に openssl で読み戻すテストまである（L1224-1312）。
  > 対象ファイル: `relay-agent/src/lib.rs`

- **WAN を TLS + mTLS + `default_permission: deny` + role CN ACL にした（継続）** `+3`
- **Relay Agent の Allocation 検証（RS256 固定・iss / aud / relay_id / role / room_jti）（継続・拡張）** `+1`
- **`/internal/*` を Tunnel ingress で 404（継続）** `+1`
- **prod / release 前提の Tunnel 手順と `/dev/*` の ingress 遮断（継続）** `+2`
- **WAN の Zenoh 接続ヘルパを `zenoh-connect` に共通化（CSR 経路含む）（継続・拡張）** `+2`
- **ローカル / WAN / WAN-e2e 設定の分離（継続・`config.wan.e2e.json5` 追加）** `+2`
- **WAN の運用手順（継続）** `+1`
- **「zenohd はトークンを見ない」境界の明記（継続）** `+1`
- **`TRUSTED_PROXIES` の必須手順化（継続）** `+2`

---

## 横断評価層

### テスト戦略

- **WAN 否定系 E2E（client 証明書なしの接続失敗・ワイルドカード購読で他室の暗号文は届くが自室鍵で開けない）を CI ジョブ化（新規・x0070）** `+4`
  > `e2e-compose.yml` の `wan-negative-e2e` ジョブ（L95-123）が mTLS zenohd を立てて `trust-probe` の WAN モードを回す。「wildcard を阻止する」と嘘をつかず「届いても読めない」を否定系で確認する作りは、保証表の文言（trust-boundary.md の x0070 行）と完全に一致している。前回の「adversarial E2E が実 client 経路を通らない」「WAN mTLS / ACL が一度も CI で動かない」の 2 点を同時に解消。
  > 対象ファイル: `.github/workflows/e2e-compose.yml`, `client/trust/src/bin/trust-probe.rs`, `docker-compose.e2e.wan.yml`, `.scripts/smoke-compose-wan-e2e.sh`
- **cargo-fuzz（protobuf decode + 上限、Room Token verify）を CI で短時間実行（継続）** `+3`
- **ヘルメティックな Compose 縦 E2E を GitHub Actions で実行（継続）** `+3`
- **auth-server の厚いテスト（継続・当日 130 件 PASS）** `+3`
- **Rust のユニットテスト（継続・当日 client 275 / protocol 67 / launcher 8 / relay-agent 16 件 PASS）** `+3`
- **relay-agent のテストを 0 → 16 件（NotAfter 読み戻し・413・503・jti 再利用 401・room_jti 否定系）（新規・x0068）** `+2`
- **adversarial E2E（継続）** `+2`
- **テスト環境の汚染を「汚染パスを渡しても無視されること」で証明する隔離テスト（継続）** `+2`
- **host Room Token の否定系マトリクス（継続）** `+1`
- **Rust と Elixir の共有テストベクタ（継続・DemoScene seal に拡張）** `+2`
- **共有 JWKS キャッシュのテスト（時計を注入）（継続）** `+1`

### 可観測性

- **loopback 専用 Prometheus テキストを host / client / assets に加え auth / world / relay まで（更新・x007a）** `+4`
  > 6 プロセスすべてが依存なしで `/metrics` を専用 loopback に出す。world は `token_issue_errors_total` を「サーバー側の失敗だけ」数え、クライアントの authz 拒否を混ぜない（`room_controller.ex` L250-276）。ノイズ削減のレビュー（`224203f`）まで含めて設計が一貫。
- **Room Token `jti` を world / relay / host のログ相関キーにした（新規・x007a）** `+2`
  > world は発行時に `room_jti` と `alloc_jti` を同じ行に出し（`room_controller.ex` L207-211）、relay は `allocate ok room_jti=… alloc_jti=…`（`relay-agent/src/lib.rs` L719-726）、host は `bind ok: sub=… room_jti=…`（`main.rs` L574）。分散トレーシング基盤なしで 1 回の入室を grep で追える。
- **auth の構造化失敗ログと telemetry（継続）** `+2`
- **host の周期カウンタ（継続）** `+1`

### エラーハンドリング

- **Rust は anyhow + context、Elixir はタグ付きタプル（継続）** `+2`
- **ストレージ障害と「存在しない」の分離（継続）** `+2`
- **応答から `inspect(reason)` を除去し、分類コードのみ返す（継続・検疫 `public_reason` に拡張）** `+1`
- **relay-agent の lock 取得を `map_err` で `Result` 化（部分・x0068）** `+1`
  > 前回指摘の `expect("jwks lock")` / `to_str().unwrap()` は本番経路の大半で `map_err` / `ok_or_else` になった（`lib.rs` L328-331, L507-555）。`jwks_age()` だけ残っている点はマイナス側に含めた。

### 変更容易性・保守性

- **保証単位のファイル分割と、モジュールごとの Guarantees / Non-guarantees（継続・x0076 で強化）** `+3`
- **ClientIp / RateLimit / AuthClient を `shared/alchemy_*` に共有化（継続・`alchemy_rate_limit` 追加 x0071）** `+3`
  > auth の ETS 固定窓を world でも使えるよう切り出した。前回の改善方針どおり。
- **JWKS キャッシュと JWT 検証を `alchemy_auth_client` に共通化（継続）** `+2`
- **Scene 分類の単一正本（`alchemy-shared-sim`、SlotGraph に拡張）（継続）** `+2`
- **ゲート（gen/rev・decode 上限・AEAD）を protocol に置き host / client で同じ関数を使う（継続・拡張）** `+2`
- **`undocumented_unsafe_blocks = "deny"`（継続）** `+1`

### 開発者体験（DX）

- **README → compose → cargo run、`make ci` を単一入口に（継続・README は 〜x007a まで追従）** `+3`
- **dev / release の 2 ステージ Docker と prod compose（継続）** `+2`
- **`make ci` が launcher / relay-agent の fmt・clippy・deny・build・test を GitHub と同一手順で回す（新規・x0068 / x0073）** `+1`
- **`make ci` が Windows ネイティブ（Git Bash + Docker Desktop）で完走（継続・当日 assets のみフレーク）** `+1`
  > 当日初回は assets がフレークで FAIL、再実行で緑。前回 `+2` から 1 段下げた。
- **prod compose の config 検証（継続・当日 PASS）** `+1`
- **Rust CI に `Swatinem/rust-cache`（継続）** `+1`

### セキュリティ・配布可能性

- **Untrusted host 前提の信頼チェーン（seal + attestation + gen/rev + discard）に、部屋機密性（AEAD）と短命 mTLS が乗った（更新）** `+4`
  > アプリ署名で「改ざん」を、AEAD で「盗聴」を、mTLS + ACL で「未認証 peer と guest の sim 偽装 put」を分担させ、それぞれの壁が何を保証しないかを trust-boundary.md に書き分けている。host Token の運び方の穴はマイナス側。
- **依存の脆弱性管理（cargo deny を client / launcher / relay-agent の 3 lock で CI ゲート、hex.audit）（更新・x0073）** `+2`
- **Untrusted 入力の入口防御（AEAD → 署名 → JWT → caps → finite → クランプ）（継続・拡張）** `+2`
- **release ビルドから既定資格情報と Token バイパスを除去（継続）** `+2`
  > `room_token_env_bypass_from(…, cfg!(debug_assertions))` と release で閉じるテスト（`credential_chain.rs` L715-730, L801-809）。
- **Zenoh `host_control` 既定 OFF、launcher はローカル IPC（継続）** `+2`
- **prod compose の公開ポートを `${PUBLISH_ADDR:-127.0.0.1}` 既定に（新規・x0073）** `+1`
  > auth / world / assets の 3 ファイルで統一（前回 -1 を closed）。

### プロジェクト全体設計

- **評価 → 改善計画 → スライス → PR の自己改善サイクルが、前回 P0 5 件・P1 5 件をすべて 1 週間でコードに落とした（継続・成果）** `+4`
  > x0066〜x0073 で P0-1〜P0-5 / P1-1〜P1-5、x0074〜x007a で P2 の大半を閉じた。各スライスに「保証する / しない」とレビュー修正コミットが付く。
- **trust-boundary.md の継続保守（`wire_cap` 秘匿を非保証へ移し、第1壁 / 第2壁を書き分け）（継続・改善）** `+4`
  > 前回「過大主張」と指摘した箇所を、実装（AEAD）を入れたうえで「wildcard で列挙されうる」と正直に書き直した。保証表を実装に合わせて弱める判断ができるのはこのプロジェクトの最大の長所。
- **architecture/*.md の「保証する／しない」表とコードの一致（継続・一部遅れ）** `+3`
- **4 階層マルチライセンス（Apache-2.0 / MPL-2.0 / Proprietary / BSL-1.1）と注入境界（新規）** `+2`
  > `.cursor/rules/license-and-boundaries.mdc` の層定義どおり、検疫と Spot-Check の詳細ヒューリスティックを Behaviour 注入点の外に出し、OSS ツリーには既定 Worker / Noop だけを置いた。
- **inventory ladder を Wave 単位でスライス化し、実装まで到達（更新）** `+2`
- **旧 Elixir 権威実行からの転換（継続）** `+2`
- **改善計画のクローズ状況を計画書側に追記する運用（継続）** `+1`
- **コンポーネント単位でパスフィルタした CI（継続・relay-agent / shared rate limit / WAN E2E を追加）** `+1`
