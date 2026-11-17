# Alchemy — 提案詳細（第1評価者 Claude Opus 5.5 / 2026-11-18）

> 評価対象コミット: `57ca452`。提案は「現時点で存在しないが、入れるとプロジェクトの価値が上がるもの」で、加点も減点もしない（`0`）。
> weaknesses の改善方針（減点の解消手順）とは重ねず、それ以外の「次の一歩」を挙げる。前回提案のうち実装されたもの（scene overlay 分離・kick API・PoP nonce・scene の Grant 照会・XPBD 接触・AUTH_PUSH・jitter・Desktop 分割・CI ボリューム分離）は外した。
> 第2評価者（`gpt/`）の文書は参照していない。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| 0 | 現時点では存在しないが、実装すればプロジェクトの価値を高める提案 |

---

## 運営側コア

### world-server

- **Permissions UI と `editor_subs` の HTTP 管理経路** `0`
  > `eligible_host_subs` には `PUT …/eligible_hosts` があるが、`editor_subs`（canonical を書ける人）を owner が編集する HTTP 経路と UI が無い。HostDelegation の権限表は world 側で完成しているので、Dash の Room タブ（x00b6）に「Editors / Eligible hosts / Denied」の 3 リストを並べると、Resonite の Permissions パネル相当になる。kick の拒否リスト（weaknesses）と同じ画面に置くのが自然。

- **署名付き HostDelegation（委譲内容を JWT に載せて host が自己検証）** `0`
  > trust-boundary.md の world 行が非保証として挙げている項目。現在は host が world に毎回問い合わせて権限を確かめる。委譲（誰が editor か・overlay 上限など）を world 署名の短命 JWT にして Room Token と一緒に渡せば、host は world が一時的に落ちていても判断でき、監査ログにも残しやすい。

- **Room 鍵の push 型 epoch ヒント** `0`
  > x00d2 で 15 秒ポーリングは軽量 API になったが、ポーリング自体は残る。ローテ時に world から Zenoh の `…/host_control` 相当（署名付き・Room AEAD 外）で「epoch が進んだ」ヒントだけを流せば、kick の効きが最大 15 秒 → 1 往復になり、平常時の world 負荷もゼロになる。

- **verifier Spot-Check のオーケストレーション（目標）** `0`
  > `AuditEngine` Behaviour の注入点はある。`sim_state`（x00b0）が「ある tick の物理状態を署名付きで取り出せる」ようになったので、Spot-Check の入力は揃いつつある。再シミュレーションの要否（mvcc_xpbd §6 Q2）を決めたうえで、`spot_check/4` を呼ぶ側の配線を OSS に置き、統計判定は Proprietary に残す。

- **S2S 連合・World 一覧の連合交換・`alchemy-node-info.json`（目標）** `0`
  > Origin Cert v2 のオフライン検証が入ったので、連合先が assets に問い合わせずに作者性を確かめられる下地はできた。

### assets-server

- **Origin Cert の署名鍵を複数保持し、`kid` で過去鍵を引く** `0`
  > 現在は `kid` を署名対象に含めるだけで、検証は現行の公開鍵で行う（`vcs/cert.ex` L47-50 の非保証）。鍵をローテした瞬間に過去の commit / release 証明が検証できなくなるので、連合を始める前に「cert 用 JWKS（現行 + 退役鍵）」を公開し、`kid` で引けるようにしておく。

- **Spatial VCS の clone / fetch / branch / diff / merge（目標）** `0`
  > push・negotiate・object 正規形・作者証明までは入った。次は「他人の公開 repo を fork して自分の Grant に取り込む」経路（fetch + 子 blob の Grant 移譲）。ここで Origin Cert の作者性が初めて意味を持つ。

---

## 共通契約層

### protocol/ ・ client/shared-sim

- **既定経路（EpochXpbd）の確保回数テストと、SceneBudget 上限での時間予算テスト** `0`
  > MVCC 経路の `…_page_alloc_bounded` と同じカウンタアロケータ方式を `EpochXpbd::step_dt` にも当て、「1 tick の確保回数 ≤ K」と「n = 4096 で 1 tick ≤ X ms」を固定する。新規依存なしで `#[global_allocator]` をテストバイナリだけに差せる。world の SceneBudget と host の物理予算が同じ数字で縛られるようになる。

- **LBVH を Karras 方式（並列構築）と refit に** `0`
  > 現在の Flat LBVH は morton 順の葉を 2 個ずつ束ねる。Karras（2012）の基数木構築は並列化でき、tick 内のサブステップは refit（AABB の更新だけ）で済む。Quest の 4 コアで効く。

- **Dual-AST Kernel / Dual Representation（目標）** `0`
  > 決定論 SIMD（ビット一致テスト済み）と MVCC page が揃ったので、ユーザースクリプトの実行器を同じ page に対するシステム関数として載せる設計がしやすくなった。

- **audio / tracking / client_info の proto（目標）** `0`

---

## ホストユーザー端末

### client/host-server ・ client/app

- **`sim_state` を HostSupervisor の planned handover に配線** `0`
  > `HostBootMode`（`ALCHEMY_HOST_BOOT_MODE`）と quarantine 付きの `sim_state` は host 側にあるが、app から設定する箇所が無い。handover accept（x00a1）の直後に、旧 host の最新 `sim_state` を新 host の起動引数に渡せば、物体が「投げた途中」のまま引き継がれる。`planned_handover_continues_*` テスト（`sim.rs` L3226-3323）が既にあるので、配線だけで価値が出る。

- **`interp.wgsl` を実際にディスパッチする GPU 補間** `0`
  > `compute_interp.rs` は naga 検証とレイアウト検査だけで、パイプラインを作っていない。CPU の jitter クロック（`sim_jitter.rs`）の α を uniform にして `cs_interpolate_entities` を回せば、entity 数に比例する補間コストを GPU に逃がせる。Quest の CPU 予算に直結する。

- **host の dual-publish 停止（Room frame を lod / cell だけに）** `0`
  > 既定の client は Room `…/frame` を購読しないが、host は Room frame を出し続けている（`bus/frame.rs` L38 の非保証）。Zenoh は購読者の無いキーをルーターまで運ばないことが多いが、opt-out 利用者がいない部屋では Room frame の封緘コスト自体を止められる。

- **Wake-on-Join・貸し出し host プール・OSC Device Input Edge・LOD ビルダー・空間オーディオ（目標）** `0`

- **Windows 配布物のコード署名と自動更新** `0`
  > 継続提案。OS vault（x00cf）を入れたので、未署名バイナリが資格情報ストアにアクセスする構図になった。SmartScreen と資格情報の両面で、署名の価値が上がっている。

---

## 転送・公開エッジ

### relay-server / relay-agent / edge

- **mTLS 証明書を部屋に束縛（SAN に room_id / L7 プロキシ）** `0`
  > 継続提案。cell / lod でキーが増えたぶん、ACL のワイルドカード（`room/*/*/…`）で「他の部屋の暗号文を購読できる」面も広がった。Room AEAD があるので平文は守られるが、トラフィック解析と帯域の両面で部屋束縛の価値が上がっている。

- **ACL のキー一覧と E2E の検査項目を機械的に突き合わせる** `0`
  > `config.wan.json5` の `key_exprs` を抽出し、各キーに「許可される役割」「拒否される役割」の E2E 検査があるかをスクリプトで確認する。今回の「cell キーは ACL にあるが E2E に無い」を、人の注意に頼らず CI で見つけられる。

- **マルチテナント名前空間・CCU / 帯域 QoS（目標）** `0`

---

## Optional Profile

- **BillingProvider / market-server（目標・BSL-1.1）** `0`
  > ディレクトリ未設置。導入時は直下に BSL-1.1 の `LICENSE` を置く（ライセンス境界ルール）。

---

## 横断

### テスト戦略・DX

- **packed revision と overlay CAS のプロパティベーステスト** `0`
  > 「どの順で append / delete / save / takeover / dormant を並行に打っても、packed revision は単調・合成 scene は SceneBudget 内・overlay 上限で fail-closed」という不変条件は、例示テストより性質テストに向く。StreamData は既存依存に無いので、まず手書きのランダム列生成（シード固定）から始めればよい。

- **ローカル CI の所要時間と失敗ステップを記録する** `0`
  > `ci.sh all` は Windows で十数分かかる。ステップごとの時間と初回失敗を `target/ci-report.json` 等に残せば、今回のような「初回だけ落ちる」を傾向として追える。

### プロジェクト全体設計

- **スライス完了条件に「trust-boundary.md の該当行」を常設** `0`
  > 継続提案（前回は「文書 grep 更新」）。今回は各スライスの保証表は丁寧なのに、横断の正本（trust-boundary）への反映だけが漏れた。新しいワイヤ・HTTP・IPC を足すスライスでは、PR テンプレートに「trust-boundary の行」をチェック項目として入れる。
