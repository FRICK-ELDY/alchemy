# Alchemy — 提案（0 点）詳細（第1評価者 Claude Opus 5.5 / 2026-11-05）

> 評価対象コミット: `c491016`。未実装の目標機能は減点せず、ここに「次のステップ」として記録する。
> 新規依存（Cargo / Hex）が必要なものは guardrails に従い「人に回す」と明記する。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では存在しないが、実装すればプロジェクトの価値を高める提案 |

---

## 門番・信頼境界

### 権限の運び方

- **制御面（spawn / despawn / 将来の編集操作）を session 鍵署名に統一し、Room Token を bind 時の 1 回だけにする** `0`
  > Intent はすでに「Token は bind 時のみ・以後は Ed25519 session 署名」（x0023 / x003e）。inventory 制御面も同じ形にすれば、bearer Token が共有鍵バスに載る構造そのものが消える。host 側は bind 済み `sub` と world から受け取る `owner_sub` で role 判定する。参考: MLS の「メッセージは送信者鍵で署名、グループ鍵は機密性のみ」。
  > 対象ファイル: `client/app/src/inventory.rs`, `client/host-server/src/inventory_spawn.rs`, `client/host-server/src/inventory_despawn.rs`

- **world の host 専用 API に `cnf.spk` の所持証明（PoP）を要求する** `0`
  > `host_keys` / `manifest_seal` / `scene/entries` へのリクエストに、`cnf.spk` の秘密鍵で `method|path|body_blake3|ts` を署名したヘッダを付ける（RFC 9449 DPoP の簡易版）。OTP `:crypto` の `eddsa` 検証で足り、新規 Hex 依存は不要。Room Token 漏えい＝即 host 乗っ取りの連鎖を断てる。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`, `world-server/lib/world/token.ex`

### 部屋鍵・証明書

- **Room 鍵の epoch 化（visibility 縮小・kick・定期でローテーション）** `0`
  > `room_key_epoch` を AAD に含め、host は現行 + 1 世代前だけ受理。Token 応答に epoch を載せ、client は epoch 変化で再取得。公開範囲 ACL の実効寿命を Token TTL まで縮められる。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `protocol/src/wire_aead.rs`

- **mTLS 証明書を部屋に束縛する（SAN / CN に `room_id` + `wire_cap` ハッシュ、zenohd 側は room 単位ルールを動的生成）** `0`
  > 現状の証明書は role だけを表すので、host 証明書 1 枚で全部屋の `inventory/*` と sim キーに put / subscribe できる（機密性は AEAD で守られるが、DoS と「どの部屋が活動中か」のメタデータは漏れる）。zenohd の ACL はファイル再読込で動的更新できないため、短期は「Relay Agent を L7 プロキシ化」、長期は zenoh の `access_control` を room テンプレートで生成する外部コントローラが候補。overview の目標「名前空間隔離 `alchemy/tenant/{運営者ID}/room/{room_id}/**`」と同じ方向。
  > 対象ファイル: `relay-server/config.wan.json5`, `world-server/lib/world/mtls_issuer.ex`

- **relay-agent を CSR 専用にするか退役させる** `0`
  > world が CSR 署名の正経路になった以上、`key_pem` を返す互換経路を残す理由は薄い。残すなら `csr_pem` 必須・非 loopback は TLS 背後でなければ起動拒否。
  > 対象ファイル: `relay-agent/src/lib.rs`

---

## 運用・性能

- **BLAKE3 / seal-hash Port をプール化（`PartitionSupervisor` で N 本）** `0`
  > 新規依存なしで、アップロードと seal の直列化上限を外せる。world の seal は部屋をまたいで共有されるので、優先度は world 側が高い。
  > 対象ファイル: `assets-server/lib/assets/blake3/worker.ex`, `world-server/lib/world/manifest_seal_hash/worker.ex`

- **`/ready` エンドポイント（Repo・署名鍵・Port・JWKS の疎通）を auth / world / assets に** `0`
  > `/health` は liveness に残し、Docker / Proxmox の healthcheck を `/ready` に向ける。seal Port 停止で全 host が配信停止する障害を検知できる。

- **host の物理 tick を `HostLoop` 構造体 + `fn tick()` に切り出し、pose バッファをダブルバッファ化** `0`
  > ユニットテスト可能になり、毎 tick の `Vec` 確保も消える。Bevy の `Schedule` / Godot の `_physics_process` に相当する境界を明示する。
  > 対象ファイル: `client/host-server/src/main.rs`

- **verifier の Spot-Check 呼び出し側を配線（Noop のまま）** `0`
  > `World.Verifier.spot_check/4` の呼び出し元（frame サンプリング → AuditEngine → 失効）だけ OSS で用意しておけば、Proprietary 実装は注入だけで済む。Zenohex Adapter（x006a）がすでにあるので購読側は揃っている。
  > 対象ファイル: `world-server/lib/world/verifier.ex`, `world-server/lib/world/network/zenoh/`

---

## テスト・品質

- **プロパティベーステスト（Rust: `proptest`、Elixir: `StreamData`）** `0`
  > リポジトリ内に proptest / StreamData は無い。`wire_aead` の seal/open 往復、`generation_gate` の単調性、Quota 予約・解放の不変条件（`used + reserved ≤ limit`）は性質テストに向く。新規依存のため**人に回す**。

- **AEAD 開封と inventory JSON を cargo-fuzz の対象に加え、fuzz workflow の paths に `protocol/**` を入れる** `0`
  > 現 `host-fuzz-ci.yml` は `client/host-server/**` 変更時のみ走る。protocol の decode / AEAD 変更でも回るべき。
  > 対象ファイル: `.github/workflows/host-fuzz-ci.yml`, `client/host-server/fuzz/`

- **`ci.sh all` に `shared/alchemy_*` 3 パッケージを追加** `0`
  > Docker の Elixir イメージで `mix test` を回すだけ。GitHub と手元の差分を消す。

- **host 乗っ取り連鎖の否定系 E2E（盗んだ host Token で `host_keys` を叩くと 401 / 制御面に Token が載らない）** `0`
  > 上の PoP / session 署名を入れたあと、trust-probe に「別 sub の client が spawn 暗号文を購読しても Token を得られない」ケースを足す。

---

## プロダクト（目標機能・減点対象外）

- **5 人以上の部屋（user_root 帯の幅を Manifest で配る）** `0`
- **Dual-AST Kernel / Wake-on-Join / OSC 受信エッジ / LOD ビルド / 空間オーディオ** `0`
  > overview の目標。コードが無いため減点しない。
- **billing / market-server（BSL-1.1）** `0`
  > ディレクトリ未設置のため評価対象外。新設時は直下に BSL-1.1 `LICENSE`。
- **`alchemy://` ディープリンクとホスト権限委譲（launcher）** `0`
- **Origin Cert の署名ペイロードに内容ハッシュと親 commit を入れた v1 形式** `0`
  > 連合・マーケット（License Token）で外部検証する前に形を固める。
