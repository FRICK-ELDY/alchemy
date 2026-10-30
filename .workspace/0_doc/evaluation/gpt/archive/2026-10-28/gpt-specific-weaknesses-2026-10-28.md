# Alchemy 第2評価者 — 具体的な弱み（2026-10-28）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 軽微な改善余地 |
| -2 | 重要な機能・設計の欠如 |
| -3 | バグ・クラッシュ・性能劣化を起こしうる明確な欠陥 |
| -4 | 価値命題を損なう重大な欠如 |
| -5 | 根幹を揺るがす致命的欠陥 |

## 運営側コア

### assets-server

- **CAS hash を知る全認証ユーザーが他人の Blob を取得できる** `-4`
  > `show` は Bearer の存在しか使わず、`conn.assigns.auth_sub` と `BlobRef.owner_sub` を比較しない。Manifest から hash が分かる設計なので、非公開・有料 asset の confidentiality を owner metadata が保証しない。BlobGrant が将来目標でも、現在の API が private content を配信できない実害は残る。
  > 対象ファイル: `assets-server/lib/assets_web/controllers/blob_controller.ex`

- **global BlobRef が object と owner reference を混同する** `-2`
  > 別 owner が同じ bytes を upload すると既存 owner の row をそのまま返し、新 owner の reference も quota 使用量も記録しない。CAS put 後の DB insert failure には object cleanup もない。条件付き quota race は直ったが、object/reference/補償 transaction の境界は未解決。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

### world-server

- **ManifestSeal endpoint が owner host 用の署名 oracle になっている** `-4`
  > world は永続 scene を保持しているのに、seal 発行時は host が送った任意の `revision` と 64桁 hash を owner token だけで署名し、DB の scene から canonical Manifest を再構築・照合しない。Untrusted host は任意 entity/hash/NaN を含む body を world 正規 seal 付きで client に信頼させられ、「server=定義、host=定義に基づく実行」を破る。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`

## 共通契約・ホスト

### protocol

- **Zenoh の untrusted payload に decode 上限がない** `-4`
  > Input/Manifest/Frame/Presence は payload byte 数、repeated entity 数、string 長を確認せず `prost::Message::decode` する。Zenoh ACL/CCU 制限がないため、到達可能な publisher が巨大 protobuf で host/client のメモリと CPU を消費できる。署名検証は decode・allocation の後なので DoS 防御にならない。
  > 対象ファイル: `client/host-server/src/main.rs`

### client/host-server

- **物理 thread が同期 HTTP/publish に依存する** `-2`
  > 60Hz loop 内で credential HTTP refresh と Manifest/Presence/Frame の `.put(...).wait()` を実行する。HTTP は一層5秒、複数 due は直列で、Zenoh backpressure も physics を停止させる。tick deadline、catch-up、bounded network queue がない。
  > 対象ファイル: `client/host-server/src/main.rs`

- **per-sub 入力頻度制限がない** `-2`
  > signature、seq、magnitude は検証するが、1 publisher が高頻度の正当 Intent を送り続ける場合の token bucket がない。input thread、mutex、ログ、physics state update を合法的に飽和できる。
  > 対象ファイル: `client/host-server/src/session.rs`

- **host JWKS cache に hard expiry がない** `-2`
  > soft TTL 超過後も cached key で Room Token を受理し、refresh failure 時の最大 stale 期限がない。world signing key が侵害・撤去された場合、world endpoint を妨害できる攻撃者は旧鍵で新規 Room Token を署名し続けられる。
  > 対象ファイル: `client/host-server/src/room_token.rs`

## クライアント・公開エッジ

### client/app

- **署名済み Manifest/Presence の replay を revision で拒否しない** `-4`
  > Manifest と Presence は署名を検証するが、最後に受理した revision/host generation を保存して単調性を強制しない。bus 上の攻撃者は有効期限内の古い Manifest を反復 replay して client を旧 host key に戻し、現 host Frame を拒否させられる。古い Presence replay も slot 誤表示や再bindを起こす。
  > 対象ファイル: `client/app/src/main.rs`

- **asset 失敗を無関係な Coin fixture で隠す** `-2`
  > download failure または hash mismatch 時に、対象 entity と異なる fixture を表示する。本番で破損・改ざん・認可失敗をユーザーが識別できず、world 定義と見た目が一致しない。
  > 対象ファイル: `client/app/src/main.rs`

- **主要 HTTP client に明示 timeout がない** `-2`
  > UI処理自体は background thread へ移ったが、auth、room create/visibility/token、asset download は default `ureq` agent を使う経路が多い。停止した endpoint で job/token refresh/download が長時間完了せず、cancel・retry・状態遷移も持たない。
  > 対象ファイル: `client/app/src/world_api.rs`

### launcher

- **host restart 後に既存 client の IPC address が更新されない** `-1`
  > launcher は新 host の address を得るが、起動済み client は旧 `ALCHEMY_HOST_IPC_ADDR` のままで、Exit Client+Host が機能しない。既知の非保証として文書化されているが、crash recovery の一部が壊れる。
  > 対象ファイル: `launcher/src/main.rs`

### zenoh-router / edge

- **WAN TLS は server-auth のみで publisher identity/namespace ACL がない** `-3`
  > 盗聴は防いだが mTLS、Room/tenant key ACL、帯域/CCU limit がなく、接続できる peer は任意 room key に publish/queryable を出せる。application signature がない payloadのDoS・replay面は router で隔離できない。
  > 対象ファイル: `zenoh-router/config.wan.json5`

## 横断品質

### テスト戦略

- **world local CI が前回実行の test DB に汚染される** `-2`
  > Docker Compose は `world-server/` を `/app` に mount し、`mix test` は `priv/data/world_test.db` を再利用する。2026-10-28 の再実行では前回の room `local` が残り、create test が 201 ではなく 409、49 tests 中1 failureとなった。repository root context では `world-server/.dockerignore` も build context ignore として効かない。test setup でDBを初期化するか、run固有 path/volumeを使う必要がある。
  > 対象ファイル: `world-server/config/test.exs`

- **E2E が実 client の trust/replay/描画境界を通らない** `-2`
  > Compose E2E は大きな改善だが e2e-probe 中心で、client が seal/attestation/revision、R2 redirect/hash、GPU state を正しく扱う縦経路は自動 gate でない。今回の replay と fixture fallback は通過する。
  > 対象ファイル: `.github/workflows/e2e-compose.yml`

- **untrusted decoder/token/input の fuzz・property test がない** `-1`
  > protobuf/JWT/JWKS/GLB/room key は adversarial input 面だが、example unit test 中心で cargo-fuzz/proptest/StreamData corpus がない。
  > 対象ファイル: `protocol/src/lib.rs`

### 可観測性

- **本番向け metrics/export と SLO がない** `-2`
  > host tick drift/queue depth、Frame age/resync、JWT/JWKS stale age、CAS latency/quota reject、S2S ACL latency を横断して収集する exporter がない。ログだけでは分散障害と攻撃を定量的に切り分けにくい。
  > 対象ファイル: `client/host-server/src/main.rs`

### 開発者体験・文書整合

- **README の現状表示が x0020/x0021 のまま** `-1`
  > 実コードは x0037 まで進み、WAN は TLS、launcher/shared-sim/visibility が実装済みだが、冒頭は「現状 x0020」「空き ID x0021」、smoke 説明には WAN TCP 表記が残る。architecture 正本は新しいため、最初の導線だけが誤解を生む。
  > 対象ファイル: `README.md`

**弱み合計: -40**
