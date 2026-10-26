# Alchemy 第2評価者 — 具体的な強み（2026-10-26）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている |
| +2 | 一般的なベストプラクティスに沿う良い設計 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る |
| +4 | プロダクション級 OSS と比較しても遜色ない |
| +5 | 個人プロジェクトとして卓越している |

## 運営側コア

### auth-server

- **アカウントの全ライフサイクルを一貫したトランザクション境界で扱う** `+4`
  > 登録・ログインだけでなく、メール検証、パスワードリセット、変更時の全 Refresh Token 失効、退会、Access Token 失効まで実装している。特にリセット処理は行ロックと DB トランザクションを併用しており、単なるデモ認証を超える。
  > `Repo.transaction`、`Ash.Query.lock("FOR UPDATE")`、`revoke_all_refresh_tokens` が協調する（`auth-server/lib/auth/accounts.ex:205-225, 459-474, 510-515`）。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`

- **RS256・kid・JWKS ローテーションを実装している** `+3`
  > Active key と検証猶予中の複数鍵を同時公開し、JWK thumbprint を `kid` に採用する。重複 kid を起動時に拒否するため、鍵切替時の曖昧さも抑えている。
  > `all_pems = [active_pem | verification_pems]`、`ensure_unique_kids!`、`signers_by_kid`（`auth-server/lib/auth/token/keys.ex:54-71`）。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`

- **列挙耐性と多軸レート制限を両立する** `+3`
  > ログインは IP＋identifier、登録は IP＋email、Refresh は IP＋token family で制限し、forgot/resend はアカウント存在有無にかかわらず同一文言を返す。認証失敗の内部理由をログへ、外部には定型 401/403 を返す分離も良い。
  > `check_rate_limit(:refresh, :token, token_key(refresh_token), limits)`（`auth-server/lib/auth_web/plugs/rate_limit.ex:67-76`）。
  > 対象ファイル: `auth-server/lib/auth_web/plugs/rate_limit.ex`

### assets-server

- **メタデータ・Quota・不変バイナリを分離した CAS 境界** `+2`
  > BLAKE3 をキーとし、DB には BlobRef/Quota のみ、バイト列は local/S3 backend に閉じ込めている。バックエンド切替も facade 経由で、HTTP 層へ保存方式を漏らしていない。
  > `def put_if_absent(blake3_hex, data), do: backend().put_if_absent(...)`（`assets-server/lib/assets/cas.ex:11-20`）。
  > 対象ファイル: `assets-server/lib/assets/cas.ex`

- **S3/R2 直配信を短命署名 URL へ分離する** `+2`
  > API が永続 URL を契約せず、local は proxy、S3 は 302 presigned GET とするため、大容量アセットが BEAM を経由し続けない。クライアント側も cross-host redirect 時に Authorization を引き回さない。
  > `ExAws.S3.presigned_url(... expires_in: ttl)`（`assets-server/lib/assets/cas/s3.ex:51-61`）。
  > 対象ファイル: `assets-server/lib/assets/cas/s3.ex`

### world-server

- **Room Token を短命・用途限定の署名能力として設計する** `+2`
  > `sub / room_id / role / jti` に加え、`iss / aud / exp` を Joken の検証契約に含め、RS256 署名する。空間計算を world-server に戻さず、門番に限定した責務は設計方針と一致する。
  > `add_claim("room_id", ..., &valid_room_id?/1)` と `add_claim("role", ..., &(&1 in @roles))`（`world-server/lib/world/token.ex:50-59`）。
  > 対象ファイル: `world-server/lib/world/token.ex`

- **公開面と Bearer 必須 API の境界が明瞭** `+2`
  > `/health` と JWKS のみ公開し、Room 作成・Token 発行は `:user_auth` pipeline に限定する。旧 Phoenix Channel や独自 UDP を持ち込んでいない。
  > `pipe_through [:api, :user_auth]`（`world-server/lib/world_web/router.ex:11-17`）。
  > 対象ファイル: `world-server/lib/world_web/router.ex`

## 共通契約・ホスト

### protocol

- **ワイヤ契約をリポジトリ直下の独立 SSoT にした** `+3`
  > client workspace 外の `alchemy-protocol` を host/client が共有し、`PROTOCOL_PIN` で世代を可視化する。生成型とキー式ヘルパを同じ crate に集約しており、コピーされた DTO の漂流を防ぐ。
  > `pub const PROTOCOL_PIN: &str = "alchemy-protocol/0.9.0-x0019";`（`protocol/src/lib.rs:17-20`）。
  > 対象ファイル: `protocol/src/lib.rs`

- **Manifest・Frame・Input・Presence の責務分離が良い** `+3`
  > 高頻度 Frame に GLB を載せず、Manifest は BLAKE3 参照、描画境界は f64、入力は intent、Presence は `sub → entity` に限定する。変更頻度とデータ寿命に沿った分割である。
  > `double x/y/z` と delta `upserts/removes`（`protocol/proto/render_frame.proto:8-27`）。
  > 対象ファイル: `protocol/proto/render_frame.proto`

- **Room キー式と liveliness の契約をテストで固定する** `+2`
  > room ごとの manifest/frame/input/presence/control/alive を共通関数で構築し、別 room の非衝突、alive key の parse、異常 room_id を単体テストしている。
  > `assert_ne!(presence_key("alpha"), presence_key("beta"));`（`protocol/src/lib.rs:277-315`）。
  > 対象ファイル: `protocol/src/lib.rs`

### client/host-server

- **署名検証と NaN/Inf 排除を物理境界の前に置く** `+3`
  > JWKS の kid miss 時再取得、issuer/audience/expiration/room_id/role 検証を行い、その後に全空間入力の `is_finite` を確認する。Untrusted host 入力ゲートとして重要な順序を守っている。
  > `verifier.verify` 後に `input_event_is_finite`（`client/host-server/src/main.rs:352-369`）。
  > 対象ファイル: `client/host-server/src/main.rs`

- **60Hz 物理と 30Hz 配信を分離し、SoA を採用する** `+3`
  > 物理 step と配信判定を別周期にし、grabbable の handle/entity/grabbable を並列配列で保持する。Rapier の hot state をオブジェクト階層へ分散させていない。
  > `PHYSICS_PER_NET = PHYSICS_HZ / NET_HZ`（`client/host-server/src/main.rs:42-47`）、`GrabbableBoxes`（`client/host-server/src/sim.rs:59-65`）。
  > 対象ファイル: `client/host-server/src/sim.rs`

- **Presence の割当・Leave・liveliness・TTL 回収が閉じている** `+3`
  > per-sub slot を first-come で割り当て、明示 Leave、liveliness Delete、3秒 TTL の三経路で解放し、RenderFrame remove と Presence revision を同期する。切断時の幽霊 entity を具体的に処理している。
  > `reclaim_sub` と `reclaim_stale`（`client/host-server/src/main.rs:130-178`）。
  > 対象ファイル: `client/host-server/src/main.rs`

- **物理・Grab・slot 再利用の振る舞いテストが厚い** `+2`
  > 床への落下、Noclip、4 slot 独立、Grab follow/release、no-steal、slot reset を 19 個の host tests で検証する。2026-10-26 の WSL 実行で 19/19 が通過した。
  > `grab_no_steal_from_other_slot`（`client/host-server/src/sim.rs:614-627`）。
  > 対象ファイル: `client/host-server/src/sim.rs`

## クライアント・公開エッジ

### client/app

- **Desktop/OpenXR を surface 境界で吸収する** `+3`
  > HMD probe、強制 Desktop、XR session 失敗時 Desktop fallback を `surface` に閉じ、system_ui や scene にランタイム選択を漏らしていない。
  > `Err(err) => ... desktop::run(...)`（`client/app/src/surface/mod.rs:42-57`）。
  > 対象ファイル: `client/app/src/surface/mod.rs`

- **取得後 BLAKE3 検証と安全な GPU layout を持つ** `+3`
  > assets GET（R2 302 追従）後に BLAKE3 を照合し、GPU uniform/vertex は `#[repr(C)] + Pod + Zeroable` で layout 契約を明示する。アセット本体を Zenoh/host に流さない。
  > `verify_blake3(&primary, &blob_blake3)`（`client/app/src/main.rs:572-588`）。
  > 対象ファイル: `client/app/src/main.rs`

- **個人 Dash と Room state を分離した Facet 設計** `+2`
  > `DashRoot` を room frame に publish せず、Facet は UI vocabulary のみを知る。OpenXR/Room 固有知識を layout 側へ持ち込まないという層間原則を守る。
  > `DashRoot is personal / not on Room frame`（`client/app/src/system_ui/facet.rs:1-19`）。
  > 対象ファイル: `client/app/src/system_ui/facet.rs`

### zenoh-router / edge

- **local/LAN と WAN の運用差を設定で分ける** `+2`
  > LAN は multicast ON、WAN は OFF とし、WAN の公開は ufw allowlist 必須と文書化する。Router に JWT 知識を持たせない境界も明記している。
  > `multicast: { enabled: false }`（`zenoh-router/config.wan.json5:15-18`）。
  > 対象ファイル: `zenoh-router/config.wan.json5`

- **Cloudflare Tunnel の秘密情報をテンプレから分離する** `+2`
  > auth/world/assets を host 名で振り分け、catch-all 404 を置き、credentials と実 config を gitignore する。拠点ポートを直接公開しない運用手順も具体的である。
  > `- service: http_status:404`（`edge/cloudflared/config.yml.example:11-18`）。
  > 対象ファイル: `edge/cloudflared/config.yml.example`

## 横断品質

### テスト・可観測性・保守性・DX

- **Rust と主要 Elixir サービスに CI 品質ゲートがある** `+3`
  > fmt、clippy `-D warnings`、build、test、Credo、migration をサービス別 workflow とローカル単一入口に揃える。実行結果は protocol 7/7、client 55/55、host 19/19、auth 107/107、world 6/6 が通過した。
  > `cargo fmt` / `clippy` / `build` / `test`（`.github/workflows/rust-ci.yml:20-76`）。
  > 対象ファイル: `.github/workflows/rust-ci.yml`

- **入力ゲートの運用カウンタを定期ログへ集約する** `+1`
  > accepted/leaves/token failure/non-finite/full を 5 秒単位で集約し、1 packet ごとの成功ログを避ける。完全な metrics ではないが、初期診断には実用的である。
  > `input gate: accepted=... dropped_non_finite=...`（`client/host-server/src/main.rs:403-417`）。
  > 対象ファイル: `client/host-server/src/main.rs`

- **境界で Result と context を積み上げる** `+2`
  > Rust の起動・HTTP・Zenoh・GPU 境界は `anyhow::Context` を使い、Elixir controller は既知エラーを HTTP status へ写像する。main 内の失敗位置がログから追いやすい。
  > `fetch_host_token(...).context("fetch host Room Token ...")`（`client/host-server/src/main.rs:485-489`）。
  > 対象ファイル: `client/host-server/src/main.rs`

- **保証／非保証をコード冒頭まで降ろしている** `+2`
  > architecture、README、crate/module doc が同じ責務境界を繰り返し明示し、未実装目標を現実の保証と混同しにくい。初期段階の分散システムとして変更影響を予測しやすい。
  > host の Guarantees / Non-guarantees（`client/host-server/src/main.rs:1-16`）。
  > 対象ファイル: `client/host-server/src/main.rs`

- **WSL/Docker を含む再現可能なローカル入口** `+2`
  > Windows ネイティブの既知制約を隠さず、WSL2、Docker、必要パッケージ、個別 CI target、smoke 群を README から辿れる。
  > `make ci-rust / ci-protocol / ci-auth / ci-world / ci-assets`（`README.md:145-169`）。
  > 対象ファイル: `README.md`

- **定義と実行の配置が全体として一貫する** `+2`
  > protocol が定義、world が門番、host が空間実行、client が endpoint、assets が不変データ門番という大枠はコードにも現れている。旧 Elixir 空間権威や client サブルーターへの逆戻りはない。
  > 対象ファイル: `.workspace/0_doc/architecture/overview.md`

**強み合計: +61**
