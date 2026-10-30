# Alchemy — 提案（0点）詳細（第1評価者 Claude Opus 5.5 / 2026-10-30）

> 評価対象コミット: `d2d9f44`。現時点で存在しない、または目標のみの項目を「次のステップ」として記録する。加点も減点もしない。
> 新規依存（Cargo / Hex）を伴う案は guardrails に従い「人に回す」と明記する。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では存在しないが、実装すればプロジェクトの価値を高める提案 |

---

## 転送・公開エッジ

### 部屋隔離と mTLS の実効化

- **payload の Room 鍵 AEAD 暗号化（キー式隔離に依存しない）** `0`
  > world が Room Token 応答で部屋ごとの対称鍵（世代付き）を配り、Manifest / Frame / Presence / snapshot を AEAD で包む。Zenoh ACL が wildcard subscribe を防げなくても、他部屋の内容は読めなくなる。署名（完全性）と暗号化（機密性）を分けて持つ Matrix の Megolm に近い。AEAD クレートの追加が要るため **人に回す**。
  > 参考: Matrix Megolm, MLS（RFC 9420）

- **Relay Agent の L7 化（部屋単位の購読強制）** `0`
  > relay が Zenoh のセッションを代理し、Allocation Token の `room_id` / `wire_cap` 以外の宣言を拒否する。Zenoh の静的 ACL では表現できない per-room 隔離を実現する。x004e の非保証欄にある「L7 Zenoh プロキシ化」の前倒し。

- **client 側 CSR と分単位 NotAfter** `0`
  > client が鍵ペアを生成して CSR だけを送り、relay は `openssl ca -enddate` 等で `expires_in` と同じ 5〜10 分の証明書を返す。秘密鍵がネットワークに出なくなり、退室・BAN 後の残存期間も分単位になる。

- **WAN 構成の否定系 E2E（`config.wan.json5` で zenohd を起動）** `0`
  > compose に mTLS + ACL の zenohd と relay-agent を足し、「guest 証明書で manifest put 不可」「guest で `*/*/manifest` を subscribe して他部屋のサンプルが届かない」「証明書なしで接続不可」を CI で確認する。

### 名前空間・公開

- **`alchemy/dev/` から `alchemy/tenant/{operator}/` への移行** `0`
  > protocol の `room_prefix` が本番 WAN 構成でも `alchemy/dev/` を使っている。運営者単位の名前空間にすれば、将来のマルチテナント・第三者リレーで ACL を運営者ごとに切れる。

- **prod compose の bind アドレスを env 化（既定 127.0.0.1）** `0`
  > `BIND_ADDR=${BIND_ADDR:-127.0.0.1}` として、LAN 公開範囲を使う運用だけ `0.0.0.0` にする。

---

## 運営側コア

### world-server

- **rate limit の共有パッケージ化（`shared/alchemy_rate_limit`）** `0`
  > auth の `Auth.RateLimit`（ETS 固定窓 + telemetry）を `alchemy_client_ip` と同じ形で切り出し、world の Room 作成・Token 発行・seal に適用する。改善計画 P3-7 の「CI が完了しなくなる」問題は、まず BLAKE3 と切り離して rate limit 単独のスライスにすると原因を切り分けやすい。

- **verifier（Spot-Check）** `0`
  > canonical seal と gen/rev ゲートが揃ったので、P4-3 の先行条件は満たされた。world 側で Frame をサンプリングし、物理の再実行で host の改ざんを検出して Room Token を失効させる。overview の目標どおり。

- **World 一覧の公開 API と Group ACL** `0`
  > 公開範囲（visibility）は Token 発行時 ACL として実装済みだが、一覧・検索の API は無い。

### assets-server

- **BlobGrant 案 B（GET の SSoT を Grant に）** `0`
  > 入室時に room の Manifest に載る blake3 群へ短命の読取 Grant（または署名付き短命 URL）を発行し、GET 既定を Grant 必須にする。assets-server-draft.md の目標形。

- **BLAKE3 / manifest-seal-hash の常駐 Port 化** `0`
  > `Port.open({:spawn_executable, ...})` で Rust の常駐プロセスに stdin でバイトを渡し、ハッシュを返させる。NIF の脆さも外部プロセス起動コストも避けられ、新規 Hex 依存も不要。

- **予約期限切れの回収ジョブ** `0`
  > `reserved_at` 列 + 定期掃除。Oban を入れるなら **人に回す**。`Process.send_after` だけでも足りる。

- **インベントリラダー（x0054〜x0065）** `0`
  > `grant_required` PUT → item ref → shared-sim の held slot → Vault → DriveNode → Prefab → VCS → Origin Cert の順の planned 文書。現時点で実装は無いので評価対象外。順序（assets の権利台帳 → sim の slot → 永続化）は依存関係に沿っていて妥当。x0054 の着手前に、上記の部屋隔離（AEAD）を済ませておくと「持ち物を他部屋に見られる」問題を先に潰せる。

### auth-server

- **`Auth.Token.Keys` の `:persistent_term` 化** `0`
  > 弱点の改善方針と同じ。依存追加なしで 1 スライス未満。

- **ToS 同意の版管理** `0`
  > `tos_version` を user に記録し、版が上がったら再同意を要求する。client の固定 `tos_agreed: true` を直すときに合わせて。

---

## 共通契約層

### protocol/

- **user_root 帯の幅を Manifest で配る** `0`
  > `USER_ROOT_ENTITY_FIRST` と連続性規則だけを protocol に残し、天井を Manifest のフィールドにする。entity ID 空間は u32 なので、帯を `0x1000_0000..` のような高位に移せば 4 の天井を外しても box ID と衝突しない。

- **プロパティベーステスト（gen/rev ゲート・decode 上限・キー式）** `0`
  > `decide_gen_rev` の全順序性、`validate_room_id` / `validate_wire_cap` とキー組み立ての往復性は proptest 向き。proptest の追加は **人に回す**（cargo-fuzz は既にある）。

- **音声・トラッキングのチャネル** `0`
  > overview の目標。現時点で対象外。

---

## ホストユーザー端末

### client/host-server

- **物理ループの `HostLoop::tick` 化とダブルバッファ** `0`
  > `main()` から tick 本体を切り出してユニットテスト可能にし、`FrameSample` のバッファを worker と往復させて毎 tick の確保を消す。

- **Formula VM / Wake-on-Join / 貸し出しプール** `0`
  > overview の目標。現時点で対象外。

### client/app

- **予測・補間（~60–120fps）** `0`
  > 30Hz の純デルタを受けて描画側で補間するだけでも体感が大きく変わる。Photon Fusion / Netcode for GameObjects の snapshot interpolation と同じ手法。

- **XR ↔ Desktop ホットスイッチ（x0053 planned）** `0`
  > wgpu 統合（x0052）で前提が揃った。Resonite のように実行中に切り替えられると、HMD を外したときの体験が良くなる。

- **OSC 受信エッジ・空間オーディオ・LOD / KTX2** `0`
  > overview の目標。現時点で対象外。

### launcher/

- **`alchemy://` ディープリンクと host handoff** `0`
  > launcher.md の非保証。ディープリンクは OS 登録（Windows はレジストリ）が要る。IPC stale の解消と同じスライスで扱うと良い。

- **インストーラ（MSIX / WiX）と自動更新** `0`
  > 現状は ZIP 配布。テスター拡大の段階で。

---

## 横断

### 可観測性

- **Room Token `jti` を相関 ID として全サービス・host のログ metadata に載せる** `0`
  > world は Token 発行時に `jti` を Logger metadata に、host は bind 成功時に `jti` をログに、client は適用した Manifest の `(gen, rev)` と `jti` をログに出す。OTLP（opentelemetry クレート / Hex）は **人に回す** が、jti のログ相関だけなら依存なしでできる。

- **auth / world の Prometheus テキスト（loopback 専用）** `0`
  > assets の `Assets.Metrics` と同じ形を shared に切り出す。

### テスト・CI

- **新クレート / 新パッケージを CI に入れることをスライス完了条件に追加** `0`
  > launcher（前回）と relay-agent（今回）で同じ漏れが 2 回起きた。roadmap の完了条件テンプレートに「`.github/workflows` の paths と `.scripts/ci.sh` に追加済み」「cargo deny の対象」を加える。

- **CI に入らない smoke スクリプトの整理** `0`
  > 生きているものは `e2e-probe` のサブコマンドへ、腐ったものは削除。

### プロジェクト全体設計

- **`vision.md` を書くか削除する** `0`
  > overview.md の冒頭に既にビジョンがあるなら、vision.md は削除して overview に誘導する。

- **README の「現状」節を roadmap から自動生成** `0`
  > README と roadmap の現在地が毎回ずれる。roadmap の最新 done 行から README の節を生成する小さなスクリプト（PowerShell / bash）で鮮度問題を構造的に消せる。

### Optional Profile

- **billing / market-server** `0`
  > リポジトリ未設置のため評価対象外。Relay Allocation Token は x004e で土台ができたので、課金フィールドを足す余地がある。
