# Alchemy 第2評価者 — 具体的な弱み（2026-10-30）

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

- **CAS 物理オブジェクトに access を置いたため先着者が全所有者の公開方針を固定する** `-3`
  > `CasObject.access` は hash ごとに一つで、最初の PUT が `authenticated` / `grant_required` を決める。後続 owner が異なる方針を要求すると 409 になるため、同一 bytes を公開 Room mesh と private inventory の両方で使えない。既知 bytes を先に PUT すれば第三者の利用方針を妨害できる。物理 CAS は所有者・認可方針を持たず、access は BlobRef/Grant 側に置くべきである（`blobs.ex:223-270, 377-398`）。
  > 改善方針: `CasObject` を bytes/size/mime のみに戻し、用途別 access を owner ref / room grant / release grant に持たせる。GET は要求 capability と Grant を照合する。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`

## 転送・公開エッジ

### relay-agent / zenoh-router

- **「300秒証明書」の実際の NotAfter が24時間** `-3`
  > API は `expires_in=300` を返すが、OpenSSL は `-days 1` で証明書を発行する（`main.rs:22-27, 260-275, 393-399`）。クライアントが5分で再割当しても、漏えいした秘密鍵と証明書は zenohd から見れば最大24時間有効であり、「短命 mTLS」の失効境界を満たさない。CN は role のみなので、漏えい host cert はその間 host ACL を持つ。
  > 改善方針: CA 実装または OpenSSL `-not_after` 相当で秒単位 NotAfter を署名し、zenohd が検証する証明書寿命と応答 TTL を一致させる。
  > 対象ファイル: `relay-agent/src/main.rs`

- **Allocation endpoint の資源上限と鍵失効境界が不足する** `-3`
  > JWKS 更新失敗を無視して soft-stale key を無期限利用し（`main.rs:186-203`）、`jti` は読み捨てで replay 制限がない。さらに接続ごとに無制限 `thread::spawn` し、ヘッダ受理後の Content-Length body loop には64KiB上限が適用されない（`main.rs:143-157, 320-348`）。有効 Token の反復で RSA-2048鍵生成/OpenSSL subprocessを直列消費できる。
  > 改善方針: JWKS hard TTL、unknown-kid cooldown、Token/主体別 bucket、同時発行 semaphore、body の事前上限、`jti` 短期重複排除を入れる。
  > 対象ファイル: `relay-agent/src/main.rs`

## ホストユーザー端末

### launcher

- **host restart 後も既存 client の IPC address が古い** `-1`
  > launcher は再起動 host の新 address を得るが、client は起動時 env の旧 `ALCHEMY_HOST_IPC_ADDR` のままである。コード自身も非保証と明記しており、crash recovery 後の「Exit Client+Host」が壊れる（`main.rs:147-153, 231-239`）。
  > 改善方針: launcher の固定 loopback discovery endpointを設け、host generation/addressをclientへ再通知する。
  > 対象ファイル: `launcher/src/main.rs`

## 横断品質

### テスト戦略

- **adversarial E2E が実 client の検証経路を通らない** `-2`
  > e2e-probe は fake/stale/oversize Manifest 後に「host が Frame を出し続ける」ことを確認するだけで、GUI client の seal/attestation/generation gate/leave抑止を通さない。ファイル冒頭も cryptographically valid replay・GUI discard・Frame署名を非保証としている（`e2e-probe/src/main.rs:3-12, 434-516`）。
  > 改善方針: 描画不要の client trust consumer を app から抽出し、偽 seal、署名済み旧 generation、oversize、unknown kid を Compose 上で実受信させる。
  > 対象ファイル: `client/e2e-probe/src/main.rs`

- **relay-agent がローカルCI・GitHub Actions・単体テストから外れている** `-1`
  > `relay-agent` は独立 Cargo packageだが、`.scripts/ci.sh` と Rust CI の path/job に含まれず、現状の `cargo test` は 0 tests である。今回の TTL/JWKS/body-limit の契約違反を自動検出できない。
  > 改善方針: fmt/clippy/build/testを必須 job化し、証明書 NotAfter、stale JWKS、oversize Content-Length、同時発行上限をテストする。
  > 対象ファイル: `.scripts/ci.sh`

### 可観測性・デバッグ容易性

- **localhost metrics はあるが relay/world/auth とSLO集約がない** `-1`
  > host/client/assets の低 cardinality metrics は改善したが、Relay allocation deny/発行時間、world seal失敗、auth S2S latency、network queue/coalesceを横断して追う出口・dashboard・alertがない。公開 scrapeを避ける判断は正しいため軽微減点に留める。
  > 改善方針: loopback collectorまたはPrometheus agentで収集し、tick late、Frame age、credential hard-stale、allocation failureをSLO化する。
  > 対象ファイル: `client/host-server/src/metrics.rs`

### 開発者体験

- **README の到達点と WAN 保証が再び stale** `-1`
  > 冒頭は「現状〜x004d」「次の空きID=x0053」「WANはTLS」と書く一方、実装はx0052、BlobGrant x004f、Relay Agent mTLS/ACL x004eまで進んでいる（`README.md:9-18`）。architecture正本は新しいため、入口だけが誤誘導する。
  > 改善方針: リリース時に現行 slice・WAN保証・独立 package一覧を機械チェックする。
  > 対象ファイル: `README.md`

**弱み合計: -15**
