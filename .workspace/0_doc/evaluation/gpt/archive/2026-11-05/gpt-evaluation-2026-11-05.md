# Alchemy 第2評価者 — 総合評価（2026-11-05）

## 評価情報

### 対象

- **評価時点** `0`
  > 評価日: 2026-11-05。Git HEAD: `c491016a49eab6496a541178ea70df1952d5bc1e`（`Merge pull request #142 ... x007b-scene-despawn`）。直近は x0077〜x007b の bind source 分割、mTLS temp ACL、DemoScene vector、metrics+jti、scene despawn が順次 main へ入っている。
  > 対象ファイル: `.git`, `.workspace/roadmap/slices/`

- **評価境界** `0`
  > auth/assets/world、protocol、client host/app、launcher、relay/edge と横断品質を実コード・テストで再読した。`market-server/` は存在しないため Optional Profile は対象外。旧 Elixir 権威実行を前提にせず、相手評価者の当日文書と `opus/` 配下は参照していない。

## 総合スコア

### 採点

| 区分 | 合計 |
|:---|---:|
| プラス | **+71** |
| マイナス | **-16** |
| 提案 | **0** |
| **正味** | **+55** |

- **総評** `+3`
  > Alchemy は「中央が定義と署名、untrusted host が実行、client が独立検証」という難しい分散境界を、個人プロジェクトとして非常に高い密度で実装している。前回 P0 の NotAfter、JWKS hard stale、body/thread/jti bound、CAS policy、relay CI、client trust E2E、world rate limit、launcher IPC stale は大半がコードとテストで閉じた。一方、新しい Room AEAD は現在の他室 ciphertext を隠すが、部屋寿命で固定された鍵が過去参加者の revoke を保証しない。さらに canonical scene の同時更新が revision CAS を持たず、現在の価値命題に直結する残件である。

## 技術評価層

### auth-server

- **狭い責務で最も安定したコンポーネント** `+5`
  > User/JWT/JWKS/Social/S2S の境界が明確で、鍵 hot path の `:persistent_term` 化も fail-closed を維持する。Docker CI は format/compile/credo/audit を含め 130 tests, 0 failures。
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`, `auth-server/lib/auth_web/`

### assets-server

- **CAS policy の前回欠陥を正しく再設計** `+4`
  > CasObject から access を除去し、BlobRef/BlobGrant/PlaceRead を分けた。Quota reserve/finalize と orphan GC も安全側の再検証を持つ。初回 Docker test は一過性の SQLite busy で 29 failures だったが、同一 image の即時再実行は 168 tests, 0 failures。再現しなかったため減点には含めないが、CI 結果には両方を記録する。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`, `assets-server/lib/assets/cas/orphan_gc.ex`

### world-server

- **署名門番は強いが、可変 scene の並行更新が追いついていない** `+3`
  > canonical seal、CSR mTLS、Room Token / PlaceRead の分離は強い。反面、append/delete の read-modify-write に revision CAS がなく、host-token scope の rate limit もない。Docker CI は 116 tests, 0 failures。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/router.ex`

### protocol

- **単なる protobuf 定義を超えた共有契約** `+4`
  > decode 上限、generation gate、key helper、AEAD AAD、capability を同一 crate に集約し、67 tests が通過した。server=定義、host/client=実行という層間原則に合う。
  > 対象ファイル: `protocol/src/`, `protocol/proto/`

### client/host-server

- **権威 loop と外部 I/O の分離が成熟** `+4`
  > session署名、finite/rate/Grab gate、60/30 Hz 分離、credential/network worker、revision 単調適用は堅い。新しい inventory spawn/despawn は world DB 側の atomicity を次の保証単位として露出させた。
  > 対象ファイル: `client/host-server/src/`

### client/app

- **trust gate の共有抽出で E2E の証明力が上がった** `+5`
  > GUI と trust-probe が同じ Accept/Discard pipeline を使い、AEAD・seal・attestation・gen/rev を実際に通す。OpenXR/Desktop 切替も surface に閉じる。ただし外部 GLB の展開後資源上限は不足する。
  > 対象ファイル: `client/trust/src/`, `client/app/src/bus/`, `client/app/src/mesh.rs`

### launcher

- **最小 PM として crash / intentional stop を区別** `+3`
  > discovery invalidate→republish により前回の restart 後 IPC stale を解消した。8 tests, 0 failures。
  > 対象ファイル: `launcher/src/main.rs`, `launcher/src/discovery.rs`

### relay-server / relay-agent / edge

- **transport 門番は大幅改善したが、鍵失効単位が role より細かくない** `+2`
  > NotAfter 300 秒、default deny ACL、bounded Agent、WAN negative test は評価できる。16 relay tests も通過した。一方 CN は role のみ、ACL は wildcard、Room key は固定で、過去参加者 revoke が成立しない。Agent 互換経路の `key_pem` 返却も残る。
  > 対象ファイル: `relay-server/config.wan.json5`, `relay-agent/src/lib.rs`, `edge/cloudflared/config.yml.example`

## 横断評価層

### テスト戦略

- **unit / fuzz / hermetic / WAN negative の層ができた** `+4`
  > protocol 67、relay 16、launcher 8、auth 130、world 116、assets retry 168 が通過。client workspace は初回 118 passed / 1 flaky failure、その後全 workspace 再実行で通過した。Compose E2E 自体は今回ローカル未実行で、workflow とコードを確認した。
  > 対象ファイル: `.github/workflows/`, `client/fuzz/`, `client/trust/src/bin/trust-probe.rs`

### 可観測性・エラー

- **門番 metrics と相関 ID が主要サービスへ到達** `+3`
  > loopback-only metrics、Room `jti` / Allocation `room_jti`、hard-stale age、tick/frame age が揃う。前回の横断相関不足は閉じた。SLO 集約と alert はまだ提案段階である。
  > 対象ファイル: `relay-agent/src/metrics.rs`, `world-server/lib/world/metrics.ex`, `client/host-server/src/metrics.rs`

### 保守性・DX

- **保証単位の分割と README 鮮度が改善** `+2`
  > host room_token と client main の分割、常駐 hash Port、共有 DemoScene vector、relay/launcher 独立 CI が入った。README は x007b までの到達点を示す。ただし Windows 全 test の一過性 filesystem flake は残る。
  > 対象ファイル: `README.md`, `client/host-server/src/room_token/`, `.scripts/ci.sh`

### セキュリティ・全体設計

- **複数の防御壁を明示する設計は優秀、revoke の寿命定義が次の焦点** `+4`
  > mTLS role ACL、Room AEAD、app signature、world seal、session signature、BlobGrant/PlaceRead が互いの非保証を補う。ただし「鍵を一度得た主体をいつ読めなくするか」が文書と実装の保証になっていない。次回は暗号の有無ではなく epoch / revoke / grace の寿命を評価すべきである。
  > 対象ファイル: `.workspace/0_doc/architecture/trust-boundary.md`, `world-server/lib/world/rooms.ex`

## 前回指摘の再判定

### まとめ weaknesses と自系統 weaknesses のコード回帰

| 前回項目 | 判定 | 再読根拠 |
|:---|:---:|:---|
| WAN wildcard subscribe / wire_cap 部屋隔離 | **partial** | wildcard は残るが Room AEAD と WAN 否定系で他室 plaintext は閉じた。過去参加者の固定鍵 revoke は新弱点へ |
| 証明書 NotAfter 24h | **closed** | `-not_after` 300s、world/Agent 両方に寿命 test |
| Allocation JWKS/body/thread/jti bound | **closed** | hard TTL、16KiB、connection/in-flight cap、jti dedupe |
| relay 秘密鍵平文返却 | **open** | Agent `AllocateResponse.key_pem` が残る。world CSR 推奨だけでは互換経路を閉じない |
| CAS access 先着固定 | **closed** | `CasObject.access` 廃止、Grant/PlaceRead へ移設 |
| relay-agent CI/test 外 | **closed** | rust-ci、local ci、16 tests、cargo-deny |
| adversarial E2E が client trust path 外 | **closed** | shared `alchemy-client-trust` + trust-probe |
| world rate limit / Room 上限 | **partial** | auth-JWT scope rate と cap は追加。host-token scope と cap race は残る |
| 相関 ID / auth-world-relay metrics | **closed** | x007a metrics + room_jti |
| launcher restart 後 IPC stale | **closed** | discovery invalidate/publish/lookup |
| README/client/vision 鮮度 | **closed** | README は x007b 到達まで更新 |
| 軽微群（reaper/Port/分割/temp/vector 等） | **partial** | x0074〜x0079 で大半 closed。prod bind 等は個別非保証として残る |

- **回帰件数（重複除外した前回まとめ 12 項目）** `0`
  > **closed 8 / open 1 / partial 3 / superseded 0**。自系統の前回 weaknesses 8 項目だけを見ると **closed 8 / open 0 / partial 0 / superseded 0**。新弱点は固定 Room key の revoke、scene update CAS、host-token rate、GLB decode budget であり、過去文書の記憶ではなく現行コードから判定した。

### x0074〜x0079 done の再確認

- **done 表記は概ねコードと一致** `+2`
  > x0074 reaper/orphan GC、x0075 permanent Port、x0076 module split、x0077 per-source bind、x0078 temp key ACL/sweep、x0079 shared DemoScene vector をそれぞれ実装・テストで確認した。done の機械的再加点はせず、回帰根拠として扱った。
  > 対象ファイル: `assets-server/lib/assets/cas/orphan_gc.ex`, `protocol/src/port_framing.rs`, `client/host-server/src/room_token/`, `client/zenoh-connect/src/`

## ローカル検証結果

### 実行結果

| 対象 | 結果 |
|:---|:---|
| `client: cargo test --workspace` | 初回 **FAIL**（client 118 pass / XR loader 1 flaky fail）→ 全体再実行 **PASS** |
| `protocol: cargo test` | **PASS**（67 tests） |
| `relay-agent: cargo test` | **PASS**（16 tests） |
| `launcher: cargo test` | **PASS**（8 tests） |
| `auth Docker CI` | **PASS**（audit/format/compile/credo/migrate、130 tests） |
| `world Docker CI` | **PASS**（audit/format/compile/credo、116 tests） |
| `assets Docker CI` | 初回 **FAIL**（SQLite busy 等 29 failures）→ 同一 image `mix test` 再実行 **PASS**（168 tests） |
| Compose hermetic / WAN E2E | 今回未実行（workflow と probe 実装を静的確認） |
| XR 実機 | 環境なし、未実行 |

- **環境制約** `0`
  > `.scripts/ci.sh` はこの Windows 環境で `bash` が利用できず起動不能だったため、Docker Compose と cargo を PowerShell から同等実行した。assets 初回失敗は再現せず、client 初回失敗も単独・全体再実行で通ったため、いずれも結果を隠さず flake として記録した。

## 結論

### 優先順位

- **P0: Room key epoch / revoke を定義する** `0`
  > 現在の AEAD は暗号学的には正しいが、認可寿命が部屋寿命と同一である。visibility と membership の価値を守るには最優先。

- **P0: world scene mutation を revision CAS 化する** `0`
  > canonical seal の定義元で更新欠落が起きれば、下流の署名が正しくても失われた状態を正当に署名してしまう。

- **P1: Agent CSR-only、host-token rate、GLB budget** `0`
  > 秘密鍵配送、持続 DoS、client OOM をそれぞれ境界で止める。

**総合: +71 / -16 = +55**
