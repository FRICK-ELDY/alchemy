# Alchemy 第2評価者 — 具体的な強み（2026-11-12）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている。問題はないが特筆するほどではない |
| +2 | 業界の一般的なベストプラクティスに沿った、良い設計判断 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る実装 |
| +4 | プロダクションレベルのゲームエンジン・OSSと比較しても遜色ない実装 |
| +5 | このクラスの個人プロジェクトでは見たことがないレベルの卓越した実装 |

## 運営側コア

### auth-server

- **アイデンティティ SSoT を狭く保った認証・鍵境界** `+5`
  > User、Argon2、RS256 access JWT、refresh token、`kid`/JWKS、失効を auth に閉じ、Room/Zenoh/asset 権限を持ち込んでいない。鍵 hot path の fail-closed、公開 JWKS、認証 pipeline 分離は小規模 OSS の水準を明確に超える。
  > 対象ファイル: `auth-server/lib/auth/token.ex`, `auth-server/lib/auth/token/keys.ex`, `auth-server/lib/auth_web/router.ex`

- **Social / S2S access と readiness を責務別に追加** `+2`
  > Follow と Friendship を区別し、world の visibility 判定は internal API に限定する。`/ready` は Repo と公開鍵集合を実検査し、単なる liveness `/health` と分けた。
  > 対象ファイル: `auth-server/lib/auth/social.ex`, `auth-server/lib/auth/ready.ex:1-57`, `.workspace/0_doc/architecture/auth-server.md:10-23`

### assets-server

- **物理 CAS・Quota・権利台帳を分離した案Bが堅い** `+5`
  > owner を持たない CasObject、owner×hash の BlobRef、読取権 BlobGrant/PlaceRead、予約式 Quota を別保証単位にしている。同一 bytes の競合、予約補償、orphan quarantine、local/S3 切替まで、Git LFS 型 CAS より権利境界が明確である。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`, `assets-server/lib/assets/cas/orphan_gc.ex`, `.workspace/0_doc/architecture/assets-server.md:20-46`

- **Origin Cert を tree/content hash に束縛した** `+3`
  > x0096 で署名メッセージを versioned canonical payload にし、Commit tree と parent を Ed25519 署名へ含めた。対象消失・内容変更・署名不一致を `:tampered` に倒すため、従来の ID だけの証明より provenance の説明力が大きく上がった。
  > 対象ファイル: `assets-server/lib/assets/vcs/cert.ex:1-40,91-188`, `assets-server/test/assets/vcs_test.exs`

- **CAS test を hermetic にし、Port を常駐プール化** `+2`
  > test では reaper/GC を止め、CAS suite を tmp_dir + async false で隔離した。BLAKE3 は PartitionSupervisor 配下の常駐 Port と `/ready` 検査へ進み、前回の SQLite/CAS フレークと直列ボトルネックを同時に扱っている。
  > 対象ファイル: `assets-server/config/test.exs`, `assets-server/test/support/cas_tmp.ex`, `assets-server/lib/assets/blake3/worker.ex`, `assets-server/lib/assets/ready.ex`

### world-server

- **host 制御面を session PoP + ledger gate に引き上げた** `+5`
  > host Room Token を inventory bus から除き、session Ed25519 署名へ置換した。HTTP は `cnf.spk` に対する method/path/timestamp/raw-body PoP、host role、`active_host_sub`、lease CASを重ねる。盗んだ bearer token 単独では canonical seal/scene を操作できなくなった。
  > 対象ファイル: `world-server/lib/world/http_pop.ex:1-104`, `world-server/lib/world_web/controllers/room_controller.ex:630-686`, `protocol/src/control_plane.rs`

- **Scene revision CAS と操作別 host rate limit を実装** `+3`
  > scene JSON 内 revision を `UPDATE ... WHERE json_extract(...)=expected` の条件にし、競合を409へ分離した。host seal/scene/PlaceRead は IP + `room_id|jti` の操作別 bucket で、前回の更新欠落と無制限署名/DB連打をコードと否定系で閉じた。
  > 対象ファイル: `world-server/lib/world/rooms.ex:188-299`, `world-server/lib/world_web/plugs/rate_limit.ex:74-167`, `world-server/test/world/scene_revision_cas_test.exs`

- **Room key epoch と host session ledger を CAS で永続化** `+3`
  > visibility 縮小・owner 手動・定期ローテで鍵を更新し、直前 epoch の host-only grace を持つ。owner と active compute host を分離し、lease expiry / handover / takeover の競合を DB 条件更新で扱う設計は、単なる in-memory host 状態より一段強い。
  > 対象ファイル: `world-server/lib/world/rooms.ex:499-550,787-1089,1399-1617`, `world-server/lib/world/rooms/room_key_grace.ex`

## 共通契約・シミュレーション

### protocol

- **ワイヤ・暗号・制御面の単一正本** `+5`
  > protobuf だけでなく、PROTOCOL_PIN、class別 decode 上限、Room AEAD AAD v2、session署名 control plane、HTTP PoP canonical bytes、generation/revision gate、key helper を独立 crate に集約した。server=定義、host/client=実行の境界がコードで共有される。
  > 対象ファイル: `protocol/proto/`, `protocol/src/decode_limits.rs`, `protocol/src/wire_aead.rs`, `protocol/src/control_plane.rs`, `protocol/src/http_pop.rs`

### client/shared-sim

- **SoA Epoch tape・世代 handle・pointer-free topology の土台** `+4`
  > `EpochSheet` は連続列、readiness/dirty summary、ABA-safe generation、Pod layout を持ち、階層は row を動かさず間接 schedule に焼く。XPBD、Flat LBVH、ArcSwap Prev/Curr を下位の汎用データ処理として分離し、固有コンテンツ概念を持ち込んでいない。
  > 対象ファイル: `client/shared-sim/src/tape.rs:1-209`, `client/shared-sim/src/topology.rs:1-96`, `client/shared-sim/src/bridge.rs:1-79`, `client/shared-sim/src/xpbd.rs`

## ホストユーザー端末

### client/host-server

- **Intent の多層 gate と Room Token 検証** `+5`
  > RS256/JWKS、iss/aud/exp/room/role、`cnf.spk`、Ed25519 seq、finite/magnitude/Grab距離、per-session + global rate、Leave/liveliness/TTLを積み重ねている。untrusted bus の入力を物理へ渡す前の責務分離は非常に強い。
  > 対象ファイル: `client/host-server/src/session.rs`, `client/host-server/src/intent_rate.rs`, `client/host-server/src/finite.rs`, `client/host-server/src/room_token/verify.rs`

- **HostLoop と network/credential worker の分離** `+3`
  > 60Hz物理、30Hz配信、credential HTTP、Zenoh encode/putを別保証単位にし、hard expiry は physics 側でも毎 tick 評価する。x0099 で tick side effect が `HostLoop` に集まり、旧 `main()` の変更影響範囲が縮んだ。
  > 対象ファイル: `client/host-server/src/host_loop.rs:1-258`, `client/host-server/src/network_worker.rs`, `client/host-server/src/room_token/credential_worker.rs`

### client/app

- **GUI と headless probe が共有する trust gate** `+5`
  > Manifest seal、HostAttestation、generation/revision、host signature、finite、decode limits、AEAD discard を `client/trust` と bus 境界で共有する。E2E が GUI と別実装の「通ったふり」にならず、受理条件そのものを試せる。
  > 対象ファイル: `client/trust/src/`, `client/app/src/bus/manifest.rs`, `client/app/src/bus/frame.rs`, `client/trust/src/bin/trust-probe.rs`

- **GLB 展開資源を入力・展開・頂点・index・pixel で制限** `+3`
  > parse前入力、buffer、image pixel、RGBA、accessor count を段階別に計上し、超過を placeholder へ落とす。BLAKE3 完全性だけでは防げない正規登録圧縮爆弾/OOMを描画境界で止めた。
  > 対象ファイル: `client/app/src/asset_decode_budget.rs:1-141`, `client/app/src/mesh.rs:138-237`, `client/app/src/assets_fetch.rs:80-88`

- **Desktop/OpenXR hot switch と MainHub session shell** `+4`
  > XR初期化失敗時のDesktop継続、READY前入力所有、F8切替、JoinとHostの分離、Waiting/Live/KeyStale/Enteringを1つの session shellで表す。デバイス差を surface に閉じ、client をリレー化していない。
  > 対象ファイル: `client/app/src/surface/hotswitch.rs`, `client/app/src/surface/xr/wgpu_session.rs`, `client/app/src/system_ui/dash.rs`

### HostSupervisor

- **launcher を廃止し、オンデマンド子プロセス責務へ縮約** `+3`
  > Joinでは起動せずHost時だけ spawnし、room retarget、ready IPC、ワンタイム token、Windows Job Object/Linux PDEATHSIG、Exit latchとspawn raceを扱う。常時二重プロセスよりプロダクト導線と責務が一致した。
  > 対象ファイル: `client/app/src/host_supervisor.rs:1-40,51-330`, `.workspace/0_doc/architecture/launcher.md`

## 転送・公開エッジ

### relay-server / relay-agent / edge

- **CSR-only短命 mTLS・default deny ACL・発行 throughput 制御** `+4`
  > Agent 応答から秘密鍵を除去し、非loopback cleartext bindを拒否した。body/connection/in-flight/jtiを制限し、検証済み sub と peer IP の短長窓を OpenSSL 前に適用する。zenohdはJWTを見ずrole CN ACLだけを持つ境界も一貫している。
  > 対象ファイル: `relay-agent/src/lib.rs:1-52,82-101,719-800`, `relay-agent/src/throughput.rs`, `relay-server/config.wan.json5`

## 横断品質

### テスト戦略

- **unit・fuzz・否定系・WAN 実hostの層が揃った** `+5`
  > 今回実行で client workspace 447、protocol 73、relay-agent 32 test が全通過した。さらに x0087 は CSR→credential worker→sealed Frame Accept と no-cert/guest put/旧epoch否定系を持ち、単純な synthetic publish から実host経路へ進んだ。
  > 対象ファイル: `client/host-server/fuzz/`, `client/trust/src/bin/trust-probe.rs`, `.scripts/smoke-compose-wan-e2e.sh`, `.github/workflows/`

### DX・可観測性

- **単一 CI 入口と shared crate 収録** `+4`
  > `.scripts/ci.sh` が client、protocol、relay-agent、3 Elixir service、`shared/alchemy_*`、prod composeを列挙し、rustfmt/clippy/deny/audit/credo/testをまとめる。旧banner grep smokeを削除し、契約 probeへ寄せた。
  > 対象ファイル: `.scripts/ci.sh:1-280`, `.github/workflows/`

- **readiness・相関ID・専用loopback metrics** `+3`
  > `/health` と `/ready` を分け、Repo/Keys/Portを依存別に検査する。room_jti、rate reject、JWKS age、tick/frame ageを持ち、metricsを公開APIポートから分離している。
  > 対象ファイル: `auth-server/lib/auth/ready.ex`, `assets-server/lib/assets/ready.ex`, `world-server/lib/world/ready.ex`, `client/host-server/src/metrics.rs`

### 全体設計・改善サイクル

- **保証する／しないをコードの単位に対応させる文書品質** `+4`
  > overviewだけでなく auth/assets/world/protocol/client/relay/trust boundary が現行コードの責務と非保証を具体化している。旧Elixir権威実行や廃止launcherも明示的に更新され、目標と現状を混同しにくい。
  > 対象ファイル: `.workspace/0_doc/architecture/`

- **前回重大指摘を約122コミットで実コードへ反映** `+5`
  > PoP、scene CAS、Agent CSR-only、host rate、GLB budget、assets hermetic、WAN host E2E、throughput、Port pool、owner cap、XR flakeを個別slice・否定系・CIまで閉じた。done表記を鵜呑みにせず再読しても、多くが実装と一致した。この速度と検証密度は個人プロジェクトとして卓越している。
  > 対象ファイル: `.workspace/roadmap/slices/x0081-control-plane-pop.md` 〜 `.workspace/roadmap/slices/x009c-bind-global-session-pub.md`

**強み合計: +82**
