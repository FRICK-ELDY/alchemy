# Alchemy — 総合評価レポート（第1評価者 Claude Opus 5.5 / 2026-11-12）

> 評価対象: `main` HEAD `8833821`（Merge PR #187 `feat/x009c-bind-global-session-pub`）
> 前回まとめ時点: `c491016`（2026-11-05）→ 今回まで **122 コミット**（x007c〜x009c・launcher 退役・HostSupervisor・Zero-Base spatial / XPBD）
> 詳細: [weaknesses](opus-specific-weaknesses-2026-11-12.md) · [strengths](opus-specific-strengths-2026-11-12.md) · [proposals](opus-specific-proposals-2026-11-12.md)
> 回帰の起点: [まとめ 2026-11-05](../archive/2026-11-05/evaluation-2026-11-05.md) · [まとめ弱点](../archive/2026-11-05/specific-weaknesses-2026-11-05.md) · [改善計画](../archive/2026-11-05/improvement-plan-2026-11-05.md) · [自系統 2026-11-05](archive/2026-11-05/opus-specific-weaknesses-2026-11-05.md)
> 第2評価者（`gpt/`）の文書は当日・アーカイブとも参照していない。改善計画の「done」注記は信じず、全判定を当日コードの再読で行った。

## 総合点

| 項目 | 点 |
|:---|---:|
| プラス合計 | **+388** |
| マイナス合計 | **-19** |
| **正味** | **+369** |

> 前回は +331 / -23 = +308。プラスは x0081〜x009c の新規分（PoP・鍵 epoch・scene CAS・Port プール・HostSupervisor・shared-sim 等）で +57。マイナスは前回の P0 4 件のうち 3 件がコード上で閉じ、残る 1 件（鍵 epoch）も大半が閉じたため大きく減ったが、**新しく入った host 移譲（x0080）と物理置換（x0093）の周辺で -3 と -2 を 1 件ずつ、GUI ホストの認証寿命で -2 を見つけた**。

---

## 観点別スコア

| ブロック | 観点 | + | - | 正味 | 前回正味 |
|:---|:---|---:|---:|---:|---:|
| 運営側コア | auth-server | +26 | 0 | **+26** | +25 |
| 運営側コア | assets-server | +35 | 0 | **+35** | +26 |
| 運営側コア | world-server | +52 | -8 | **+44** | +26 |
| 共通契約 | protocol/ | +40 | 0 | **+40** | +33 |
| 共通契約 | client/shared-sim | +4 | -1 | **+3** | （host に同居） |
| ホスト端末 | client/host-server | +48 | -2 | **+46** | +40 |
| ホスト端末 | client/app（HostSupervisor 含む） | +52 | -5 | **+47** | +41 |
| ホスト端末 | launcher/ | — | — | **対象外（退役）** | +7 |
| 転送・エッジ | relay-server / edge / relay-agent | +24 | -1 | **+23** | +20 |
| 横断 | テスト戦略 | +30 | -1 | **+29** | +24 |
| 横断 | 可観測性 | +10 | 0 | **+10** | +9 |
| 横断 | エラーハンドリング | +7 | 0 | **+7** | +6 |
| 横断 | 変更容易性・保守性 | +13 | 0 | **+13** | +13 |
| 横断 | 開発者体験（DX） | +10 | 0 | **+10** | +8 |
| 横断 | セキュリティ・配布可能性 | +17 | 0 | **+17** | +13 |
| 横断 | プロジェクト全体設計 | +20 | -1 | **+19** | +17 |
| | **合計** | **+388** | **-19** | **+369** | **+308** |

Optional Profile（billing / market-server）はディレクトリ未設置のため評価対象外。`launcher/` は git 管理から削除済み（`git ls-files launcher` = 0、ローカルに古い `target/` のみ）で、責務は `client/app/src/host_supervisor.rs` に移ったため HostSupervisor を client/app で評価した。

---

## 一言でいうと

前回「次の穴」とした 3 つ — **host Token の運び方と PoP、Room 鍵の寿命、canonical scene の原子性** — は 1 週間でコードに落ちた。しかも閉じ方が良い。制御面は Intent と同じ session 鍵署名（`control_plane.rs`）にして Token をバスから外し、world の host API には `cnf.spk` による HTTP PoP を全 5 経路に入れた。Room 鍵は AAD に epoch を入れた v2 に上げ、visibility 縮小と同じ CAS UPDATE でローテし、旧鍵は host にだけ grace で渡す。scene は JSON 内 revision に対する CAS。relay-agent は CSR 専用になり、非ループバック待受を bind 後にも拒否する。Port は `PartitionSupervisor` でプール化され、`/ready` が 3 サービスに入った。さらに launcher を削除して client を親プロセスにし、Job Object / PDEATHSIG で道連れを保証する HostSupervisor を書き切った。個人プロジェクトの改善速度としては前回に続き異例。

一方で、**権限の付け方**に新しい穴がある。x0080 で「永続オーナー」と「現計算ホスト」を分けたのは正しいが、**canonical scene の書き込み権限を現計算ホストに付けた**ため、public の部屋では誰でも dormant 中に takeover して、オーナーのワールドを恒久的に書き換えられる。定義（world）と実行（host）の分離という、このプロジェクトが最も大事にしてきた原則に、自分の新機能が反している。また、物理を Rapier から自作 XPBD に置き換えた結果、**物体同士がすり抜ける**状態で wire 権威になっている。GUI からホストを開始すると約 15 分で Room Token を更新できなくなる点も、配布ビルドの本線に関わる。

---

## 前回からの変化（回帰判定 A）

### まとめ 2026-11-05 のマイナス（25 行）

| # | 前回項目（点） | 判定 | 根拠（当日再読） |
|:---:|:---|:---|:---|
| P0-1 | host Room Token bearer + PoP 欠如（-3） | **closed** | spawn / despawn は `session_pub` + `control_seq` + `control_sig` のみで Token 無し（`client/app/src/inventory.rs` L253-305）。署名は `alchemy/control/v1|spawn|room|seq|blake3|pose` に room と単調 seq を含む（`protocol/src/control_plane.rs` L14-31）。host は開封→署名検証→Dual-auth→seq（`inventory_spawn.rs` L228-286）。world host 5 経路が `require_http_pop`（`room_controller.ex` L635, L730, L806, L901, L1017・`http_pop.ex` L35-47） |
| P0-2 | Room 鍵非ローテ / 過去参加者 revoke 不可（-3） | **partial → -1** | epoch 化は本物（AAD v2 `wire_aead.rs` L94-96・CAS ローテ `rooms.ex` L1417-1430, L1487-1500・grace `room_key_grace.ex`・24h 定期 `room_key_rotator.ex`・host dual-open `room_aead.rs` L73-145）。残件: `rotate_room_key/2` に HTTP ルートが無く kick 不可、friendship 解除ではローテしない |
| P0-3 | Scene revision CAS なし（-3） | **closed** | `cas_set_scene` が `WHERE … json_extract(scene,'$.revision') = ?`、0 行時に 404 / 409 を区別（`rooms.ex` L207-219, L242-253, L270-300）。controller は `expected_revision` 必須（`room_controller.ex` L809-811） |
| P0-4 | relay-agent `key_pem` cleartext（-2） | **closed** | `csr_pem` 必須・`key_pem` 削除をテストで固定（`relay-agent/src/lib.rs` L6, L719-724, L1737-1738）。非ループバックは起動前と bind 後で拒否（L241-277, L319） |
| P1-1 | host-token HTTP rate limit なし（-2） | **closed** | `host_seal` / `host_scene` / `place_read` を操作別、jti 軸は Bearer claims から（`plugs/rate_limit.ex` L6-7, L75-132・`config.exs` L64-75） |
| P1-2 | GLB 展開後上限なし（-2） | **partial → -1** | `AssetDecodeBudget`（`asset_decode_budget.rs` L20-28）を buffer・頂点・index・画像に課金（`mesh.rs` L203-241）。残件: 画像 pixel 判定がフルデコード後（`mesh.rs` L229-232） |
| P1-3 | assets mix test フレーク（-1） | **closed（原因側）** | test で reaper / GC を起動しない（`config/test.exs` L20-22）。当日の初回失敗は別原因（credo コンパイル時の Elixir 型検査クラッシュ）で、新規 -1 として別立て |
| P1-4 | Allocation throughput / WAN E2E 未到達（-1） | **closed** | `throughput.rs`（短窓 + 長窓・ロールバック）、WAN 実 host 一往復 + 旧 epoch 否定（`smoke-compose-wan-e2e.sh` L2, L263・`e2e-compose.yml` L96-118） |
| P2 | BLAKE3 / seal Port 直列 | **closed** | `PartitionSupervisor` プール（`assets/blake3/worker.ex` L4-44・`world/manifest_seal_hash/worker.ex`） |
| P2 | mime 先着 | **closed** | octet-stream + nosniff 固定（`blob_controller.ex` L36-50） |
| P2 | Origin Cert が内容非束縛 | **closed** | v1 `…|tree_blake3|parent_commit_id`（`vcs/cert.ex` L14-23, L222-236） |
| P2 | `/health` が Port 非反映 | **closed** | `/ready`（`health_controller.ex` L10-27）。auth / assets にも |
| P2 | 共有 JWKS 失敗クールダウン無し | **closed** | 失敗も cooldown、miss は cache のみで GenServer をブロックしない（`alchemy_auth_client/jwks.ex` L8-12, L21-22, L245） |
| P2 | `USER_ROOT_SLOTS_MAX=4` | **closed** | x009b（帯 `0x1000_0000..`・天井 32・幅は Manifest） |
| P2 | host `main()` 肥大 | **closed** | `HostLoop::tick`（`host_loop.rs` L172-260）、`main()` は配線のみ（`main.rs` L430-759） |
| P2 | 配信 tick `Vec` | **closed** | `PoseScratchPool`（`network_worker.rs` L10-17, L143-165） |
| P2 | bind 自己申告キー | **closed** | x009c（`session_pub` 起点 + soft-global） |
| P2 | take-out UI と guest Token の不整合 | **closed** | host Dual-auth `owner_sub`（`inventory_spawn.rs` L255-271）、world `GET scene` が `owner_sub` を返す（`room_controller.ex` L715-745） |
| P2 | dash / desktop 一枚岩 | **open（悪化）→ -2** | `dash.rs` 2268 → 3604 行、`desktop.rs` 1815 → 3805 行 |
| P2 | 腐った smoke | **closed** | x0089 で削除・ci-e2e へ吸収（当日 `.scripts` に banner-grep smoke 無し） |
| P2 | `ci.sh` が shared 未収録 | **closed** | `ci.sh` L147-228, L257, L266（当日 shared 3 パッケージ PASS） |
| P2 | client.md / wan / world 保証表鮮度 | **partial → -1** | x008a で一度閉じたが、x0093 後も overview / client.md / protocol.md が Rapier、README / release workflow に launcher 残骸 |
| P2 | `vision.md` 空 | **closed** | x008a |
| P2 | Room 上限の並行 overshoot | **closed** | quota 行 + insert 単一 TX（`rooms.ex` L1267-1290・`owner_quota.ex`） |
| P2 | Windows XR loader flake | **closed** | 一意 temp パス（`surface/xr/loader.rs` L242-250）。当日 client workspace は初回 PASS |

**集計（まとめ 25 行）: closed 21 / partial 3 / open 1 / superseded 0**

### 自系統 2026-11-05 のマイナス（20 件）

| # | 前回項目 | 判定 | 根拠 |
|:---:|:---|:---|:---|
| 1 | BLAKE3 Port 直列（-1） | closed | 上記 P2 |
| 2 | mime 先着（-1） | closed | 上記 P2 |
| 3 | Origin Cert 内容非束縛（-1） | closed | 上記 P2 |
| 4 | host Token bearer + PoP 欠如（-3） | closed | 上記 P0-1 |
| 5 | Room 鍵非ローテ（-2） | partial | 上記 P0-2（-1 で残置） |
| 6 | Room Token 系 RL 無し + seal 直列（-1） | closed | 上記 P1-1 / P2 |
| 7 | `/health` 非反映（-1） | closed | 上記 P2 |
| 8 | 共有 JWKS クールダウン（-1） | closed | 上記 P2 |
| 9 | USER_ROOT 天井（-1） | closed | x009b |
| 10 | host `main()` 一枚岩（-1） | closed | x0099 |
| 11 | 配信 tick `Vec`（-1） | closed | x0095 |
| 12 | bind 自己申告キー（-1） | closed | x009c |
| 13 | take-out UI が host Token 前提（-1） | closed | x0098 |
| 14 | `dash.rs` / `desktop.rs` 肥大（-1） | **open（悪化）** | -2 に引き上げ |
| 15 | relay-agent `key_pem`（-1） | closed | 上記 P0-4。`curl` / lock `expect` は新規 -1 で別立て |
| 16 | assets `mix test` 並列フレーク（-1） | closed | 上記 P1-3（当日は別原因フレーク） |
| 17 | 腐った smoke（-1） | closed | x0089 |
| 18 | `make ci` が shared / fuzz を覆わない（-1） | partial | shared は closed。fuzz は `host-fuzz-ci.yml` で GitHub 側にのみ。ローカル入口が別なだけなので減点しない |
| 19 | 保証表・設定コメント鮮度（-1） | closed → 新規 | x008a で閉じたが、x0093 / launcher 退役で新しいずれ（-1 新規） |
| 20 | `vision.md` 見出しのみ（-1） | closed | x008a |

**集計（自系統 20 件）: closed 17 / partial 2 / open 1 / superseded 0**

### improvement-plan の「done」注記の当日確認

| ID | 計画上の状態 | 当日確認 |
|:---|:---|:---|
| P0-1 制御面 + PoP（x0081 + x0098） | done | **確認** |
| P0-2 Room key epoch（x007e） | 計画書上は推奨順 2 のまま（done 注記なし） | **実装済み（PR #150）**。ただし kick API 未配線で partial |
| P0-3 Scene CAS（x0082） | done | **確認** |
| P0-4 Agent CSR-only（x0083） | done | **確認** |
| P1-1〜P1-5（x0084〜x0088） | done | **確認**（P1-2 は画像デコード順のみ partial） |
| P2-1〜P2-14（x0089〜x009c） | done | **確認**（P2-10 docs は再び部分的にずれ） |

計画書側の P0-2 は実装済みなのに「推奨スライス順 2」として未完了のまま残っており、計画書の更新漏れがある（コードが先行している側のずれなので減点はしない）。

---

## 当日の重大度上位マイナス（トップ 5）

1. **Takeover した第三者がオーナーの永続 canonical scene を書き換えられる（定義と実行の逆転）** `-3`（新規）
2. **GUI からホストを開始すると約 12〜15 分で host が Room Token を再発行できなくなる** `-2`（新規）
3. **Rapier → 自作 XPBD 置換で body-body 接触・Grab 回転追従が失われた** `-2`（新規）
4. **`dash.rs` / `desktop.rs` の肥大が倍増** `-2`（継続・悪化）
5. **world の軽微群（catalog 無制限・scene append が Grant を見ない・PoP の skew 内リプレイ）** `-2`（新規）

---

## ローカル CI / テスト実行結果（2026-11-12・Windows 11 ネイティブ + Git Bash + Docker Desktop）

コマンド: `./.scripts/ci.sh all`（Git Bash、約 16 分）→ 失敗ステップのみ `./.scripts/ci.sh assets` で再実行

| ステップ | 結果 | 備考 |
|:---|:---|:---|
| client `cargo fmt --check` / `clippy -D warnings` / `cargo deny` / build | PASS | |
| client `cargo test --workspace` | PASS | 447 tests（前回 275）。XR loader flake は再発せず |
| relay-agent fmt / clippy / deny / build / test | PASS | 32 tests（前回 16） |
| protocol `cargo test` | PASS | 73 tests（前回 67） |
| protoc schema compile | PASS | |
| auth（Docker：hex.audit / format / compile -Werror / credo --strict / test） | PASS | 133 tests, 0 failures |
| world（Docker：同上） | PASS | 162 tests, 0 failures（前回 116） |
| assets（Docker：同上） | **FAIL（初回）→ 再実行 PASS** | 初回は依存 credo のコンパイル中に Elixir 1.19.5 型検査が `Credo.Check.ConfigComment.__info__/1 undefined` でクラッシュ（テスト未到達）。単独再実行で 191 tests, 0 failures。**フレーク**と判定（開発用 assets コンテナが同じ `assets_build` ボリュームで起動中だった） |
| shared/alchemy_*（Docker） | PASS | auth_client 13 / client_ip 5 / rate_limit 7 |
| prod compose config（x003b） | PASS | |
| **RESULT（初回）** | **FAILED（assets のみ）** | 再実行で全緑相当 |

実行しなかったもの（環境制約）:

- `make ci-e2e`（hermetic Compose E2E）と `wan-host-e2e`：Linux ランナー前提の長時間ジョブのため未実行。GitHub Actions の結果は取得していない（静的にジョブ定義とスクリプトの否定系を確認したのみ）。
- `make ci-fuzz`：nightly + Linux / macOS 前提。
- XR 実機（OpenXR ランタイム）での手動確認。

---

## git の状態

- HEAD: `8833821`（Merge PR #187）
- 前回まとめ `c491016` から 122 コミット。主な流れ:
  - x007c〜x007f：Worlds catalog / Join・Host、MainHub session shell、live Manifest mesh remount、prefab 多子 inventory
  - x007e：Room 鍵 epoch（AEAD v2）
  - #144 / #151：HostSupervisor（launcher 置換）と UI auth handoff
  - x0080：host handover / takeover + dormant
  - x0081〜x0088：制御面 PoP、scene CAS、relay-agent CSR-only、host RL、GLB budget、assets hermetic、WAN host E2E、Allocation throughput
  - x0089〜x009c：smoke 整理 / shared CI、docs 鮮度、`/ready` + nosniff、JWKS cooldown、Port プール、Zero-Base spatial（tape / topology / LBVH / XPBD / FrameBridge）、client Epoch → SceneGpu、XPBD wire 権威、pose ダブルバッファ、Origin Cert v1、Room 上限厳密化、take-out owner session、HostLoop::tick、XR loader、USER_ROOT 帯、bind session_pub
- 差分規模: client/app +11.5k 行、client/host-server +4.7k / -2.1k、client/shared-sim +3.1k、world-server +4.3k、launcher -2.3k（削除）
- 作業ツリー: 前回評価文書のアーカイブ移動（`evaluation/*-2026-11-05.md` → `archive/2026-11-05/`）のみ未コミット。本評価では評価文書以外を変更していない。

---

## 技術評価層の所見（ブロック別講評）

### auth-server（+26 / 0）

変更は `/ready` のみ。Refresh ファミリー回転・多軸 rate limit・列挙対策は引き続き最強部品。新規の欠陥なし。

### assets-server（+35 / 0）

前回の 3 件（Port 直列・mime 先着・Origin Cert 非束縛）をすべて閉じた。Origin Cert を連合前に `v1|…|tree_blake3|parent` で固定し、検証失敗を `:tampered` で fail-closed にした判断は、git の署名付きコミットよりも一段厳格。Drive の put_back が子ごとに Grant を見る点も良い。当日マイナスなし。

### world-server（+52 / -8）

門番としての厚みは前回から最も伸びた（PoP・鍵 epoch・scene CAS・操作別 RL・lease CAS・`/ready`）。その一方で、**最大の欠陥（takeover host による canonical scene 改ざん）もここ**。x0080 の lease 設計自体は CAS で丁寧なのに、権限表の 1 行（scene 書き込み = active host）がプロジェクトの根本原則とぶつかっている。`rooms.ex` 1734 行・`room_controller.ex` 1476 行という肥大も、権限表の見落としが起きやすい構造の一因と見る。epoch 配布が全員 15 秒ポーリングという点も、15 人集会を目標にするなら早めに push 型へ。

### protocol（+40 / 0）

AEAD v2（epoch AAD）、制御面署名、HTTP PoP の canonical bytes が加わり、「decode 上限 → AEAD → 署名 → gen/rev」の多層を純関数で共有する設計がさらに厚くなった。前回同様、このプロジェクトで最も完成度が高い層。

### client/shared-sim（+4 / -1）

前回までは名前だけだった shared-sim が、SoA の `EpochSheet`・Flat LBVH・XPBD・FrameBridge を持つ本物のクレートになった。方向性はプロジェクトの DOD 規則どおりだが、肝心の tick ループで全シート clone・毎サブステップ確保・HashSet 彩色が残る。規則を最も守るべき場所で守れていない。

### client/host-server（+48 / -2）

`HostLoop::tick` の切り出しと pose プールで前回の保守性負債を両方閉じた。Room AEAD の epoch 単調採用、HTTP PoP クライアント、Dual-auth の順序（未 bind が共有 rate に届かない）も丁寧。ただし XPBD 置換で物体同士の接触が無くなり、使っていない `rapier3d` が依存に残っている。

### client/app（+52 / -5）

HostSupervisor はプロセス境界の設計として非常に丁寧（Job Object・PDEATHSIG・exit latch・無ロック ready 待ち）。GLB budget、epoch probe、Epoch → SceneGpu も積み上がった。しかし (1) GUI ホストが access JWT 失効で止まる、(2) `desktop.rs` / `dash.rs` が倍増した、(3) 画像 budget がデコード後、の 3 点が残る。(1) は配布ビルドの本線で、回避策が「パスワードを環境変数に」なのは避けたい。

### launcher（対象外）

中途半端に残さず削除し、HostSupervisor に移したのは良い判断（設計で +1）。README と release workflow に残骸がある。

### relay-server / relay-agent / edge（+24 / -1）

`key_pem` を消し CSR 専用・ループバック限定にして前回の残件を閉じた。主体別 throughput も入った。残るのは `curl` 子プロセスと lock `expect` の軽微な点のみ。

### 横断

- **テスト**: client 447・world 162・assets 191・auth 133・protocol 73・relay 32。WAN 実 host 一往復に旧 epoch 否定を入れ、「新しい保証は否定系 E2E で」を守っている。ただし takeover → scene 書き込みの**否定系テストが無い**ことが、今回の -3 を見逃した直接の原因。assets のローカル CI フレークは原因が変わって再発。
- **可観測性**: `/ready` が 3 サービスに揃い、発行ログに `room_key_epoch` が入った。
- **保守性**: 新規モジュールも Guarantees / Non-guarantees を書く運用は守られている。一方で world と client/app の一枚岩化が進んだ。
- **セキュリティ**: 前回「最後の 1 ピース」とした PoP が入り、Untrusted host 前提の信頼チェーンは共有鍵モデルの範囲で閉じた。次は「誰に何の権限を渡すか」（scene 書き込み・kick・Grant 照会）の権限表。
- **全体設計**: 自己改善サイクルは前回 P0〜P2 の大半を 1 週間で閉じた。ただし新機能（x0080 / x0093）を入れたスライスで、保証表に権限と機能後退が書かれていない。

---

## 同種 OSS との比較

| 軸 | 比較対象 | 所見 |
|:---|:---|:---|
| 署名モデル | Matrix server signing | 中央定義 → 署名 → 失敗は破棄に、制御面署名と HTTP PoP が加わり水準が揃った |
| 部屋機密 | MLS / Matrix E2EE | epoch 導入で「入室 = 永久読取」は解消。kick / 退室即時失効は未（MLS の Remove 相当なし） |
| 権限モデル | Resonite / VRChat のインスタンス権限 | **ワールド保存（定義）とセッション内操作（実行）の分離が崩れている**。Resonite は Host でもワールド保存先の所有者権限が無ければ上書きできない |
| 物理 | Rapier / avian（旧 bevy_xpbd） | SoA・二重シートは良いが、接触・回転・確保ゼロで後退 |
| プロセス | Steam クライアントの子プロセス管理 | Job Object / PDEATHSIG まで書いた点は同種 OSS 以上。認証更新の受け渡しが未 |
| CAS / 権利 | git LFS / IPFS | Grant / PlaceRead + Origin Cert v1 で権利モデルは一段上。scene append の Grant 照会が残る |

---

## 次の一手（第1評価者の推奨順）

1. **scene の永続書き込みを owner（と明示 editor）に限定し、takeover host はセッション overlay だけを持つ**。否定系テスト（非オーナー takeover → scene append/delete が 403）を同じ PR に入れる。
2. **HostSupervisor から host へ access JWT を IPC で push**（refresh token は渡さない）。GUI 起動ホストが 15 分で止まる問題を閉じる。
3. **XPBD の body-body 接触を host で有効化し、tick ループの確保をゼロに**（`clone_from` / 彩色マーカー / LBVH refit）。使わない `rapier3d` を外し、overview / client.md / protocol.md を XPBD に更新。
4. **kick API（`rotate_room_key` の HTTP 化 + deny list）と epoch の push ヒント**。全員 15 秒ポーリングをやめる。
5. **`desktop.rs` / `dash.rs` をイベント → コマンドに分割**、world に `FallbackController` と `Rooms` の分割、CI 用 Elixir ボリュームの分離。

---

## 評価の前提

- 旧 Elixir 権威実行前提は使っていない。
- 目標のみ（verifier 統計・billing / market・Dual-AST Kernel / OSC・Wake-on-Join 等）は減点せず提案または対象外とした。
- 第2評価者の文書は読んでいない。改善計画の「done」注記は参照のみで、判定はすべて当日コードの再読に基づく。
- assets の初回失敗は再実行で消え、恒常バグではなくフレークとして記録した。
