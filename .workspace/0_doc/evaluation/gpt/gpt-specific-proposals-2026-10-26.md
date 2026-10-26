# Alchemy 第2評価者 — 具体的な提案（2026-10-26）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では未実装・評価対象外だが、実装すれば価値を高める提案 |

## 運営側コア

### auth-server

- **Friendship / Group / ToS version migration の実装** `0`
  > 現在の User SSoT を維持したまま、Friendship と Group membership を auth domain に追加すると、world の Friends/Friends+ 公開範囲を文字列 ACL ではなく署名済み `sub` 関係から判定できる。未実装目標なので減点しない。
  > 対象ファイル: `auth-server/lib/auth/accounts/`

### assets-server

- **CAS object と owner/license reference の分離** `0`
  > `CasObject(hash,size)` と `BlobGrant(owner_sub,hash,rights)` を分け、global dedupe を維持しつつ GET/Quota を user reference 単位にする。将来の License Token、group Vault、購入品容量 0 と自然に接続できる。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

- **署名付き direct PUT と検疫 pipeline** `0`
  > upload reservation で quota を原子的に確保し、短命 presigned PUT 後に HEAD/hash/GLB header を検証して commit する二相方式を提案する。大容量 upload の BEAM 経由と orphan を同時に減らせる。
  > 対象ファイル: `assets-server/lib/assets/cas/s3.ex`

- **Spatial VCS / Origin Cert を別保証単位で追加** `0`
  > Vault/Commit/Branch/Tag/Origin Cert は CAS 門番と責務が異なるため、既存 `Blobs` に肥大化させず domain/module を分ける。現状コードがないため評価対象外。
  > 対象ファイル: `assets-server/lib/assets/`

### world-server

- **永続 Room capability と5段階公開範囲** `0`
  > Room owner、member/invite capability、role escalation policy を永続化し、Token 発行は capability 判定結果だけを署名する。Friends/Friends+ は auth の関係を参照し、world が User 本体を複製しない。
  > 対象ファイル: `world-server/lib/world/`

- **Verifier を独立 application として実装** `0`
  > Spot-check、host lease、Room Token `jti` denylist、host 移譲を world の HTTP controller へ混ぜず、独立 supervisor/application にする。コード未設置の目標なので減点しない。
  > 対象ファイル: `world-server/`

## 共通契約・ホスト

### protocol

- **互換性テスト用 golden corpus と buf breaking gate** `0`
  > 各 pin の binary fixture、unknown field round-trip、最大件数、全 `.proto` の breaking check を CI に入れる。単なる compile より「旧 client と新 host が何を保証するか」を固定できる。
  > 対象ファイル: `protocol/`

- **audio / tracking を別帯域契約として追加** `0`
  > AudioFrame と TrackingFrame は RenderFrame に混ぜず、codec/version/rate/sequence を持つ独立 schema と key にする。現状未実装はロードマップ上の対象外。
  > 対象ファイル: `protocol/proto/`

### client/host-server

- **simulation と network publish の bounded pipeline 化** `0`
  > simulation thread は 60Hz state を更新し、network worker が reusable snapshot buffer から 20/30/60Hz delta を生成する。遅延時は古い未送信 delta を統合し、physics を止めない。
  > 対象ファイル: `client/host-server/src/main.rs`

- **Formula VM / Wake-on-Join / snapshot handoff** `0`
  > deterministic Formula VM、0人時 sleep、host lease と snapshot sequence を shared-sim 保証単位として追加する。現状コードがないため減点しない。
  > 対象ファイル: `client/host-server/`

## クライアント・プロセス管理

### client/app

- **jitter buffer と補間・限定予測** `0`
  > sequence を検証する小さな jitter buffer を置き、remote entity は interpolation、local user_root は intent replay/reconciliation を行う。現在の 30Hz pose 直適用から 60–120fps の体感へ進める。
  > 対象ファイル: `client/app/src/scene.rs`

- **署名 Manifest と host frame chain の検証** `0`
  > world 署名 Manifest、host lease id、frame sequence/hash chain を検証してから SharedState に反映する。Zenoh transport ACL と application-level authority の双方を持つべき。
  > 対象ファイル: `client/app/src/main.rs`

- **OSC Device Input Edge と Dual Editor** `0`
  > OSC は localhost listener に限定し、address allowlist、finite/rate validation 後に compact tracking schema へ変換する。Dual Editor は共通 AST を別 crate に置き、UI node と WGSL/Formula text が同じ定義を編集する。いずれも未実装目標なので減点しない。
  > 対象ファイル: `client/app/src/`

- **暗号化 cache と LOD bundle manifest** `0`
  > BLAKE3 検証済み blob を content key で保存し、PC/Quest/Web の LOD variant と decoder budget を manifest に含める。単なる難読化を権利保護と表現せず、鍵失効・再取得の保証を明記する。
  > 対象ファイル: `client/app/`

### launcher

- **launcher を最小プロセスマネージャとして実装** `0`
  > 現在 `launcher/` は空で評価対象外。第一段階は client/host の spawn、PID/exit code 監視、graceful shutdown、crash loop backoff、ログ収集に限定し、その後 `alchemy://` と host handoff を追加する。
  > 対象ファイル: `launcher/`

## 転送・公開エッジ

### zenoh-router / edge

- **mTLS/QUIC と Relay Agent による namespace capability** `0`
  > WAN zenohd の前段または plugin/sidecar で Relay Allocation Token を検証し、tenant/room prefix、publish/subscribe direction、帯域を接続単位で制限する。host の Room Token 検証だけでは frame/control 面を保護できない。
  > 対象ファイル: `zenoh-router/`

- **cloudflared HA と WAF/rate-limit policy as code** `0`
  > Tunnel replica、origin health、API path 別 rate limit、JWT endpoint の bot policy を Terraform 等で再現可能にする。現テンプレは単一 tunnel の初期運用としては妥当なため提案点のみ。
  > 対象ファイル: `edge/cloudflared/`

## 横断品質

### テスト・可観測性・配布

- **Hermetic E2E と fault injection** `0`
  > Compose で auth/world/assets/zenoh/host/headless client を起動し、鍵 rotation、packet reorder、router restart、R2 302、quota race、host crash/handoff を検証する。WAN dependency は fake S3/JWKS で再現可能にする。
  > 対象ファイル: `.github/workflows/`

- **OpenTelemetry と SLO dashboard** `0`
  > `room_id`、host lease、protocol pin、trace id を共通属性にし、tick lateness、frame age/drop、JWKS age、CAS p95、token rejection、active slots を metrics/traces へ出す。高 cardinality の `sub` は label にしない。
  > 対象ファイル: `client/host-server/src/main.rs`

- **依存・SBOM・署名を release gate にする** `0`
  > `cargo audit/deny`、`mix hex.audit`、CycloneDX/SPDX、成果物署名、provenance を CI に追加し、HIGH advisory は期限付き exception 以外 fail とする。
  > 対象ファイル: `.github/workflows/`

- **デスクトップ配布と crash report の設計** `0`
  > launcher 完成後に Windows/macOS/Linux installer、設定 migration、minidump、protocol pin の互換性診断をまとめる。未実装の配布機能なので減点しない。
  > 対象ファイル: `launcher/`

**提案合計: 0**
