# Alchemy プロジェクト総合評価 — 第2評価者 GPT-5.6 Sol

**評価日:** 2026-10-26  
**評価方式:** 初回評価。アーキテクチャ文書と実コードを直接照合し、ローカル検証を実行した。  
**総点:** **+61 / -62 = -1点**

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1〜+5 | 正しい実装から、個人プロジェクトとして卓越した実装まで |
| -1〜-5 | 軽微な改善余地から、根幹を揺るがす致命的欠陥まで |
| 0 | 未実装・評価対象外、または将来価値を高める提案 |

## エグゼクティブサマリー

Alchemy は「auth/assets/world を永続権威と門番に限定し、Pure Rust host が空間実行、client は endpoint、protocol は独立 SSoT」という分離を、目標図だけでなく動くコードまで降ろしている。RS256/JWKS、Refresh Token family、CAS/R2、Rapier 60Hz、Presence reclaim、OpenXR fallback、ローカル CI は個人プロジェクトの試作として明確に平均を上回る。

一方、現状を WAN 参加可能な分散 VR 基盤として扱うのは危険である。特に、(1) 認証なし `host_control` による遠隔終了、(2) assets の所有者認可欠落、(3) 任意 user が host role を取得可能、(4) 平文・無認証 WAN Zenoh、(5) client が偽 Frame/Presence を無検証適用、の五点は同じ trust boundary の穴として連鎖する。さらに assets のローカル CI は実際に失敗し、依存監査では HIGH advisory も確認した。したがって設計の方向性は高評価だが、現在の配布可能性は低く、正味は **-1点** とする。

## 技術評価層

### auth-server — アイデンティティ SSoT

#### ✅ プラス点

- **アカウントライフサイクルと Token 失効** `+4`
  > reset/change/deactivate と Refresh Token 全失効を transaction 内で扱う（`auth-server/lib/auth/accounts.ex:205-273`）。
- **複数 kid/JWKS による鍵ローテーション** `+3`
  > active＋verification keys を同時公開し、重複 kid を拒否する（`auth-server/lib/auth/token/keys.ex:54-81`）。
- **列挙耐性・多軸 rate limit・HTTP 境界** `+3`
  > IP/identifier/email/token family の軸を分ける（`auth-server/lib/auth_web/plugs/rate_limit.ex:45-76`）。

#### ❌ マイナス点

- **Refresh 同時実行の二重 rotation** `-2`
  > record の `FOR UPDATE` または条件付き revoke がない（`auth-server/lib/auth/accounts.ex:307-327`）。

#### 💡 提案

- **Friendship / Group / ToS version migration** `0`
  > 5段階公開範囲の根拠を auth SSoT に置く。

**小計: +10 / -2 = +8点**

### assets-server — CAS・Quota・門番

#### ✅ プラス点

- **DB メタと local/S3 CAS の分離** `+2`
  > backend facade が保存方式を隔離する（`assets-server/lib/assets/cas.ex:7-22`）。
- **R2 presigned GET への 302** `+2`
  > 大容量 download を BEAM から外す（`assets-server/lib/assets/cas/s3.ex:51-61`）。

#### ❌ マイナス点

- **owner_sub を GET 認可に使用していない** `-4`
  > hash のみで BlobRef を取得する（`assets-server/lib/assets/blobs.ex:99-103`）。
- **Quota/CAS/DB の並行整合性がない** `-3`
  > ledger read-check と increment が原子的でない（`assets-server/lib/assets/blobs.ex:140-167`）。

#### 💡 提案

- **CasObject と BlobGrant の二層化、direct PUT reservation** `0`
  > global dedupe と所有者/license 認可を両立する。

**小計: +4 / -7 = -3点**

### world-server — 案内所・門番

#### ✅ プラス点

- **用途限定の短命 Room Token** `+2`
  > `iss/aud/exp/sub/room_id/role/jti` を RS256 で署名する（`world-server/lib/world/token.ex:10-59`）。
- **公開面と Bearer API の分離** `+2`
  > health/JWKS と Room API を pipeline で分ける（`world-server/lib/world_web/router.ex:7-22`）。

#### ❌ マイナス点

- **Room ownership と host role 認可がない** `-4`
  > request の role を署名し、存在しない room も `ensure` する（`world-server/lib/world_web/controllers/room_controller.ex:27-58`）。

#### 💡 提案

- **永続 capability、公開範囲、Verifier** `0`
  > いずれも現状コードがない目標なので減点しない。

**小計: +4 / -4 = 0点**

### protocol — ワイヤ契約 SSoT

#### ✅ プラス点

- **独立 crate と PROTOCOL_PIN** `+3`
  > client 外の単一契約を共有する（`protocol/src/lib.rs:17-20`）。
- **Manifest/Frame/Input/Presence の頻度別分割** `+3`
  > Frame は f64 pose delta、Manifest は hash reference に限定する（`protocol/proto/render_frame.proto:8-30`）。
- **room key/liveliness helper と tests** `+2`
  > room 分離と parse を test vector で固定する（`protocol/src/lib.rs:277-315`）。

#### ❌ マイナス点

- **room_id validator の層間不一致** `-2`
  > Rust は printable ASCII、world は英数 `_ -`＋64 bytes である。
- **protoc CI が全 schema を対象にしない** `-1`
  > scene/presence が直接 compile list にない（`.github/workflows/protocol-ci.yml:29-37`）。

#### 💡 提案

- **golden corpus / breaking check / audio・tracking schema** `0`

**小計: +8 / -3 = +5点**

### client/host-server — Untrusted 計算ホスト

#### ✅ プラス点

- **Room Token 検証後の finite gate** `+3`
  > kid refresh、iss/aud/exp/room_id/role の後に物理入力を検査する（`client/host-server/src/room_token.rs:97-133`）。
- **60Hz physics / 30Hz network と SoA** `+3`
  > 周期を分け、grabbable を並列配列で保持する（`client/host-server/src/sim.rs:59-65`）。
- **Leave/liveliness/TTL の Presence 回収** `+3`
  > 三経路を同じ slot lifecycle に収束させる（`client/host-server/src/main.rs:130-178`）。
- **Rapier/Grab/slot の振る舞い tests** `+2`
  > WSL 実行で host 19/19 tests が通過した。

#### ❌ マイナス点

- **無認証 shutdown control** `-5`
  > Zenoh payload だけで `process::exit(0)`（`client/host-server/src/host_control.rs:20-27`）。
- **magnitude/rate limit 不在** `-3`
  > finite な巨大 move/aim を許す（`client/host-server/src/finite.rs:1-14`）。
- **同期 publish が physics tick をブロック** `-2`
  > simulation loop 内で Zenoh `.wait()` を実行する（`client/host-server/src/main.rs:724-735`）。
- **既知 seed credentials の暗黙 fallback** `-4`
  > `alice / Password1` を env 未設定時に使用する（`client/host-server/src/room_token.rs:14-17`）。

#### 💡 提案

- **bounded network pipeline、Formula VM、Wake-on-Join、handoff** `0`

**小計: +11 / -14 = -3点**

### client/app — 体感・描画・UI endpoint

#### ✅ プラス点

- **Desktop/OpenXR surface fallback** `+3`
  > runtime probe/session failureを surface 層で吸収する（`client/app/src/surface/mod.rs:21-57`）。
- **assets 直取得後の BLAKE3 と GPU layout 契約** `+3`
  > redirect 後に hash を照合し、GPU 構造を Pod で固定する（`client/app/src/main.rs:572-588`）。
- **個人 Dash と Room state の分離** `+2`
  > Facet は OpenXR/Room を知らず、Dash を publish しない（`client/app/src/system_ui/facet.rs:1-19`）。

#### ❌ マイナス点

- **Frame/Manifest/Presence を無認証・未 sanitization で適用** `-4`
  > f64 をそのまま f32 pose へ入れる（`client/app/src/main.rs:230-252`）。
- **sequence/revision/再同期不在** `-3`
  > frame tick を読まず、欠落した delta に収束手段がない。
- **box と remote の UBO alias** `-2`
  > remote が box 用 buffer を再利用する（`client/app/src/scene.rs:614-633`）。
- **UI thread の blocking HTTP** `-2`
  > click handler が login/join 完了まで描画を止める（`client/app/src/surface/desktop.rs:695-739`）。
- **hash/download failure を fixture で隠す** `-2`
  > 対象 asset ではない Coin を表示する（`client/app/src/main.rs:579-587`）。

#### 💡 提案

- **jitter buffer・補間予測・署名 frame・OSC・Dual Editor・LOD cache** `0`
  > 未実装目標は減点しない。

**小計: +8 / -13 = -5点**

### launcher — プロセスマネージャ

`launcher/` は空であり、明示ルールにより未実装・評価対象外とする。

#### 💡 提案

- **spawn/monitor/backoff/graceful shutdown を先行実装** `0`

**小計: +0 / -0 = 0点**

### zenoh-router / edge — 転送・公開エッジ

#### ✅ プラス点

- **LAN/WAN config と multicast 方針の分離** `+2`
- **Tunnel credentials と公開 template の分離** `+2`
  > catch-all 404 まで含む（`edge/cloudflared/config.yml.example:11-18`）。

#### ❌ マイナス点

- **WAN が平文 TCP＋IP allowlist のみ** `-4`
  > Frame/Presence/Manifest/control に transport identity も application signature もない（`zenoh-router/config.wan.json5:10-18`）。

#### 💡 提案

- **mTLS/QUIC、Relay Agent、tenant namespace/QoS** `0`

**小計: +4 / -4 = 0点**

### Optional Profile

billing / market-server ディレクトリは存在しないため評価対象外。減点しない。

## 横断評価層

### テスト戦略

#### ✅ プラス点

- **サービス別 CI と実用的な単体 test 群** `+3`
  > protocol 7、client 55、host 19、auth 107、world 6 tests は 2026-10-26 に通過した。

#### ❌ マイナス点

- **assets ローカル CI が 7 tests 中 1 failure** `-3`
  > S3 env が test の local backend を上書きし、GET 期待 200 が 302 となった（`assets-server/config/runtime.exs:14-26`）。
- **サービス横断 E2E が自動 gate でない** `-2`
- **fuzz/property test 不在** `-1`

#### 💡 提案

- **Hermetic Compose E2E と fault injection** `0`

**小計: +3 / -6 = -3点**

### 可観測性・デバッグ容易性

#### ✅ プラス点

- **host gate の集約カウンタログ** `+1`
  > accepted/drop reason を5秒単位で出す（`client/host-server/src/main.rs:403-417`）。

#### ❌ マイナス点

- **tick/frame/CAS/JWKS を横断する metrics・trace がない** `-2`

#### 💡 提案

- **OpenTelemetry と SLO dashboard** `0`

**小計: +1 / -2 = -1点**

### エラーハンドリング戦略

#### ✅ プラス点

- **Rust context と Elixir status mapping** `+2`
  > 起動境界の失敗理由を保持し、外部へ内部認証情報を返さない。

#### ❌ マイナス点

- **Mutex poison を空 Presence として継続** `-2`
  > invariant failure を revision 0 の正常状態へ変換する（`client/host-server/src/main.rs:600-623`）。

#### 💡 提案

- **境界別 retry/backoff/restart policy の明文化** `0`

**小計: +2 / -2 = 0点**

### 変更容易性・保守性

#### ✅ プラス点

- **保証／非保証を module 単位まで明記** `+2`

#### ❌ マイナス点

- **entity ID/slot 定数の host/client 重複** `-2`
  > protocol または shared-sim で定義すべき（`client/host-server/src/sim.rs:20-34`、`client/app/src/scene.rs:24-34`）。

#### 💡 提案

- **shared-sim crate と typed EntityKind** `0`

**小計: +2 / -2 = 0点**

### 開発者体験（DX）

#### ✅ プラス点

- **WSL/Docker/個別 CI/smoke の単一 README 導線** `+2`
  > Windows の非推奨経路も明記する（`README.md:30-169`）。

#### 💡 提案

- **devcontainer と one-command hermetic demo** `0`

**小計: +2 / -0 = +2点**

### セキュリティ・配布可能性

#### ❌ マイナス点

- **既知 HIGH/MEDIUM advisory を gate しない** `-3`
  > assets 実行時に `hackney 1.25.0` の HIGH 1件、MEDIUM 2件等を確認したが、`mix hex.audit` / `cargo audit` gate がない（`assets-server/mix.lock:16`）。

#### 💡 提案

- **audit/deny、SBOM、署名、installer/release provenance** `0`

**小計: +0 / -3 = -3点**

### プロジェクト全体設計

#### ✅ プラス点

- **定義・門番・実行・endpoint の大枠がコードと一致** `+2`
  > protocol=定義、world=門番、host=実行、client=endpoint、assets=CAS 門番という方向は正しい。

#### 💡 提案

- **trust-boundary threat model を architecture 正本へ追加** `0`
  > 「何を保証しないか」だけでなく、未認証 publisher が各 key へ到達した場合の防御責任を図示する。

**小計: +2 / -0 = +2点**

## ローカル検証結果

| 対象 | 結果 |
|:---|:---|
| `protocol cargo test`（WSL） | **成功** — 7 passed |
| `client cargo test --workspace`（WSL） | **成功** — client 55 + host 19 passed |
| `bash .scripts/ci.sh auth`（Docker） | **成功** — format/compile/credo/migrate、107 passed |
| `bash .scripts/ci.sh world`（Docker） | **成功** — format/compile/credo、6 passed |
| `bash .scripts/ci.sh assets`（Docker） | **失敗** — 7 tests 中 1 failure（local 200 期待に S3 302） |
| 依存監査相当の出力 | `hackney 1.25.0` に HIGH/MEDIUM/LOW advisory を確認 |

## 総合得点

| 評価観点 | プラス | マイナス | 正味 |
|:---|---:|---:|---:|
| auth-server | +10 | -2 | +8 |
| assets-server | +4 | -7 | -3 |
| world-server | +4 | -4 | 0 |
| protocol | +8 | -3 | +5 |
| client/host-server | +11 | -14 | -3 |
| client/app | +8 | -13 | -5 |
| launcher | +0 | -0 | 0 |
| zenoh-router / edge | +4 | -4 | 0 |
| テスト戦略 | +3 | -6 | -3 |
| 可観測性 | +1 | -2 | -1 |
| エラーハンドリング | +2 | -2 | 0 |
| 変更容易性 | +2 | -2 | 0 |
| DX | +2 | -0 | +2 |
| セキュリティ・配布 | +0 | -3 | -3 |
| プロジェクト全体設計 | +2 | -0 | +2 |
| **合計** | **+61** | **-62** | **-1** |

## 優先順位

1. `host_control` を即時無効化または署名 capability 必須にし、既定資格情報を削除する。
2. assets を CasObject/owner grant に分け、GET 認可と quota race を修正する。
3. world に room owner/role capability を導入し、任意 host token 発行を止める。
4. WAN Zenoh に相互認証・暗号化・namespace ACL を導入する。
5. client で署名/finite/size/sequence を検証し、full snapshot 再同期を実装する。
6. assets CI を hermetic に直し、dependency audit を必須 gate にする。
