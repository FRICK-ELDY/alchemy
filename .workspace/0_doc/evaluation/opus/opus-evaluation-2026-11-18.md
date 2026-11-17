# Alchemy — 総合評価レポート（第1評価者 Claude Opus 5.5 / 2026-11-18）

> 評価対象: `main` HEAD `57ca452`（Merge PR #247 `feat/x00d6-rooms-fallback-split`）
> 前回（自系統）: `8833821`（2026-11-12）→ 今回まで **135 コミット**（x009d〜x00d6 の 58 スライス）
> 詳細: [weaknesses](opus-specific-weaknesses-2026-11-18.md) · [strengths](opus-specific-strengths-2026-11-18.md) · [proposals](opus-specific-proposals-2026-11-18.md)
> 回帰の起点: [まとめ 2026-11-12](../archive/2026-11-12/evaluation-2026-11-12.md) · [まとめ弱点](../archive/2026-11-12/specific-weaknesses-2026-11-12.md) · [改善計画](../archive/2026-11-12/improvement-plan-2026-11-12.md) · [自系統 2026-11-12](archive/2026-11-12/opus-specific-weaknesses-2026-11-12.md)
> 第2評価者（`gpt/`）の文書は当日・アーカイブとも参照していない。改善計画の「closed」注記は信じず、全判定を当日コードの再読で行った。

## 総合点

| 項目 | 点 |
|:---|---:|
| プラス合計 | **+467** |
| マイナス合計 | **-15** |
| **正味** | **+452** |

> 前回は +388 / -19 = +369。プラスは x009d〜x00d6 の新規分（HostDelegation・SpawnTicket・Spatial VCS・Spatial Interest・MVCC page・edit レーン・AUTH_PUSH・OS vault 等）と、前回の弱点を閉じたことによる格上げで +79。マイナスは前回 19 点のうち 13 点分がコード上で閉じた。新たに見つけたのは、**既定物理経路の LBVH 構築が O(n²)**（-2）、**assets に流量制御が一切無い**（-2）、**新しいワイヤ経路が E2E に入っていない**（-1）など 9 点分。

---

## 観点別スコア

| ブロック | 観点 | + | - | 正味 | 前回正味 |
|:---|:---|---:|---:|---:|---:|
| 運営側コア | auth-server | +27 | 0 | **+27** | +26 |
| 運営側コア | assets-server | +41 | -2 | **+39** | +35 |
| 運営側コア | world-server | +70 | -3 | **+67** | +44 |
| 共通契約 | protocol/ | +48 | 0 | **+48** | +40 |
| 共通契約 | client/shared-sim | +11 | -3 | **+8** | +3 |
| ホスト端末 | client/host-server | +60 | -1 | **+59** | +46 |
| ホスト端末 | client/app（HostSupervisor 含む） | +67 | -2 | **+65** | +47 |
| ホスト端末 | launcher/ | — | — | **対象外（退役）** | 対象外 |
| 転送・エッジ | relay-server / edge / relay-agent | +26 | 0 | **+26** | +23 |
| 横断 | テスト戦略 | +34 | -2 | **+32** | +29 |
| 横断 | 可観測性 | +10 | 0 | **+10** | +10 |
| 横断 | エラーハンドリング | +9 | 0 | **+9** | +7 |
| 横断 | 変更容易性・保守性 | +14 | 0 | **+14** | +13 |
| 横断 | 開発者体験（DX） | +10 | -1 | **+9** | +10 |
| 横断 | セキュリティ・配布可能性 | +18 | 0 | **+18** | +17 |
| 横断 | プロジェクト全体設計 | +22 | -1 | **+21** | +19 |
| | **合計** | **+467** | **-15** | **+452** | **+369** |

Optional Profile（billing / market-server）はディレクトリ未設置のため評価対象外。`launcher/` は git 管理外で、責務は `client/app/src/host_supervisor.rs` で評価した。Proprietary（pgvector 検疫・Spot-Check 統計）は OSS ツリーに求めていない。

---

## 一言でいうと

前回の最大欠陥「takeover した第三者がオーナーの canonical scene を書き換えられる」は、1 行の権限チェックを足すのではなく、**権限モデルごと作り直して**閉じた。canonical は owner ∪ editor だけ、takeover host は捨てられる session overlay だけ、takeover できる人も owner ∪ eligible だけ。canonical と overlay は 1 つの packed revision で CAS し、否定系テストまで揃えた。GUI host の資格情報切れは「refresh は親だけが持ち、子には短命 access JWT だけを push する」broker で閉じ、平文 refresh は OS の資格情報ストアに移した。scene の Grant 照会は、assets が発行し world が 1 回だけ消費する **SpawnTicket** というサービス間の権限証票で閉じた。そのうえで Spatial VCS（決定的 CBOR・サーバー再検証・fast-forward・作者証明 v2）、Spatial Interest（cell / lod の AOI 配信）、MVCC page と決定論 SIMD、決定的 edit レーンまで 1 週間で積み上げている。改善の速さと閉じ方の筋の良さは、前回に続いて個人プロジェクトとして異例。

一方で、**新しい既定経路の足元**に穴が残る。物理は radix sort まで入れた LBVH の AABB 引き当てが線形探索で O(n²) になっており、それが 60Hz × 4 サブステップで毎回走る。world が受理する 4096 entries を host が回せない。assets はコア 3 サービスで唯一流量制御を持たないまま、12 MiB の VCS push と固定サイズの Port プールを抱えた。cell 配信を既定にしたのに取りこぼしの回復手段が無く、しかもその新経路（cell / lod / edit / sim_state）は E2E に一本も入っていない。前回「新しい保証は否定系 E2E で」を守った運用が、今回の 58 スライスでは単体・統合テストで止まった。

---

## 前回からの変化（回帰判定 A）

### まとめ 2026-11-12 のマイナス（11 行・採用 -23）

| # | 前回項目（点） | 判定 | 根拠（当日再読） |
|:---:|:---|:---|:---|
| 1 | 可視参加者の無委譲 takeover が canonical scene 改変権まで渡す（-4） | **closed** | canonical の書き込み 3 経路が `Rooms.ensure_scene_writer/2`（owner ∪ `editor_subs`・`rooms.ex` L223-245）を通る（`room_controller.ex` L637, L671, L706）。takeover host は `session_overlay` のみで、交代 / dormant で破棄 + revision 前進（`rooms/internal.ex` L43-53）。takeover は owner ∪ `eligible_host_subs`。否定系テスト `scene_writer_forbidden` 4 箇所（`room_controller_test.exs` L2097-2311）・`session_overlay_test.exs` L45 |
| 2 | UI ログイン host が約 15 分で資格情報更新不能（-3） | **closed** | 親が refresh 後に loopback IPC `AUTH_PUSH` で access JWT を push、refresh は子に渡さない（`host_supervisor.rs` L30-31, L133-185）。子は ParentBroker のときだけ適用（`credential_chain.rs` `apply_parent_auth_push`） |
| 3 | kick / friendship 解除の即時 revoke 未完（-2） | **partial → -1** | owner の `POST …/room_key/rotate`（`router.ex` L33）と auth → world `relation_changed`（L40）は入った。残件: 拒否リストが無く public 部屋では再入室で新鍵を得られる・client に kick 導線なし・通知に再試行なし |
| 4 | Rapier → XPBD 置換で body-body 接触が host で切られた（-2） | **closed** | 不等式接触（`xpbd.rs` L222, L273-304）、host は `Some(self.scene_bounds)` で接触有効（`sim.rs` L1238）、`rapier3d` を依存から削除 |
| 5 | shared-sim / host ホットパスが 60Hz で全 sheet clone と一時確保（-2） | **open → -1（継続）+ 新規 -2** | MVCC page 経路は確保 ≤ 2 / tick をテストで固定したが、既定 `EpochXpbd` は clone 3 回（`sim.rs` L1231, L1265, L1272）・`collect()`（`xpbd.rs` L55, L336）・`HashSet`（L101）のまま。さらに LBVH 構築が O(n²)（`spatial.rs` L191-211）で毎サブステップ（L429-448）を新規 -2 で別立て |
| 6 | Graceful handover が opt-in を見ず Presence 先頭を 5 分指名（-2） | **closed** | 指名は「候補」止まりで lease は動かず、候補が `POST …/handover/accept`（`router.ex` L30）して初めて active host（`session_flow.rs` L1408-1442・`HostStartKind::Nominated` L1752） |
| 7 | canonical scene の構造上限が無い（-2） | **closed** | `World.SceneBudget`（4096 entries・1 MiB JSON）を全 scene CAS 経路と合成後に適用 |
| 8 | 7 日 refresh token を平文 prefs JSON に保存（-1） | **closed** | `keyring` の `alchemy.refresh`（`refresh_vault.rs` L13, L44-58）、prefs は key id のみ、旧平文は移行、不能時 session-only（L123-156） |
| 9 | `dash.rs` / `desktop.rs` の肥大が倍増（-2） | **partial → -1** | `desktop.rs` は `surface/desktop/{mod, session_flow, inventory_flow, hotswitch_flow}` に分割、`UiAction`（`system_ui/mod.rs` L1453）。残件: `dash.rs` 3604 → 3779 行 |
| 10 | world 軽微群（catalog RL・scene append の Grant・PoP skew 内リプレイ・epoch 15s ポーリング・rooms 一枚岩）（-1） | **closed** | catalog RL（`rate_limit.ex` L63）・SpawnTicket（`world/spawn_ticket.ex`）・PoP nonce（`http_pop/nonce.ex`）・`GET …/room_key_epoch`（`router.ex` L51-52）・`Rooms` 分割 + `FallbackController`。ただし `scene/save` の RL 抜けを新規 -1 で別立て |
| 11 | GLB pixel 判定がデコード後 / KeyStale 手動 / docs・release 残骸 / assets credo フレーク / relay-agent curl+expect（-2 帯） | **partial → -2（DX -1・テスト -1）** | GLB（`mesh.rs` L236-242）・KeyStale（dual-open）・relay-agent（ureq）は closed。残件: README L117 が必ず失敗する `smoke-rapier-host.sh` を案内（DX -1）、assets ローカル CI が原因を変えて初回失敗（テスト -1） |

**集計（まとめ 11 行）: closed 7 / partial 3 / open 1 / superseded 0**

### 自系統 2026-11-12 のマイナス（13 件・-19）

| # | 前回項目 | 判定 | 根拠 |
|:---:|:---|:---|:---|
| 1 | takeover host が canonical scene を書き換えられる（-3） | closed | 上表 #1 |
| 2 | kick API 無し・friendship でローテしない（-1） | **partial** | 上表 #3（-1 で残置） |
| 3 | epoch 配布が 15 秒の Room Token 再発行ポーリング（-1） | closed | 軽量 `GET …/room_key_epoch`、同 epoch なら再発行しない（`room.rs` L20, L707-729・`credential_worker.rs` L5, L181）。push 型は提案へ |
| 4 | `rooms.ex` / `room_controller.ex` 一枚岩・エラー変換の重複（-1） | closed | `rooms.ex` 1734 → 540 行ファサード + `Rooms.{Internal,Lease,RoomKey,Scene,Catalog}`・`FallbackController` の表（L43 等） |
| 5 | world 軽微群（-2） | closed | 上表 #10 |
| 6 | XPBD ホットパスが DOD / ゼロ確保規則に反する（-1） | **open** | 上表 #5（-1 継続 + O(n²) 新規 -2） |
| 7 | XPBD 置換で接触・Grab 回転追従を喪失（-2） | closed | 上表 #4 |
| 8 | `dash.rs` / `desktop.rs` 肥大（-2） | **partial** | 上表 #9（-1 で残置） |
| 9 | GUI host が約 12〜15 分で Room Token を再発行できない（-2） | closed | 上表 #2 |
| 10 | GLB pixel 判定がデコード後（-1） | closed | `mesh.rs` L236-242 |
| 11 | relay-agent の `curl` 子プロセスと lock `expect`（-1） | closed | `ureq`（5 秒タイムアウト）・`expect` 除去。残る `Command::new` は openssl CSR のみ |
| 12 | ローカル CI の assets がコンパイラクラッシュで落ちる（-1） | **partial** | CI 専用ボリュームに分離（`ci.sh` L240-246）し前回の原因は消えたが、当日は stale `_build/test` による「redefining module」で初回失敗（-1 で残置） |
| 13 | アーキ文書・配布ワークフローに退役済み記述（-1） | **partial** | 本文は解消。残件の smoke スクリプトを DX で -1 計上 |

**集計（自系統 13 件）: closed 8 / partial 4 / open 1 / superseded 0**

### improvement-plan の「closed」注記の当日確認

| ID | 計画上の状態 | 当日確認 |
|:---|:---|:---|
| P0-1 HostDelegation + canonical owner/editor 限定 | closed（x009d + x009e） | **確認** |
| P0-2 親 client 資格情報 broker | closed（x009f） | **確認** |
| P1-1 kick API + friendship → epoch | closed（x00a0） | **partial**。計画の完了条件（旧鍵で新 epoch が読めない）は満たすが、拒否リスト・UI・通知再試行が無い |
| P1-2 host で XPBD 接触 | closed（x00a4 + x00ad） | **確認** |
| P1-3 hot-path zero-allocation | Phase 1（page-alloc）closed | **確認（MVCC 経路のみ）**。本番既定の `EpochXpbd` は未着手で、計画書自身も非保証と明記。評価上は open |
| P1-4 handover accept 二段階 | closed（x00a1） | **確認** |
| P1-5 SceneBudget | closed（x00a2） | **確認** |
| P1-6 refresh を OS vault へ | closed（x00cf） | **確認** |
| P2-1 dash / desktop 分割 | closed（x00d0） | **partial**。desktop と `UiAction` は確認、`dash.rs` 3779 行が残る |
| P2-2 catalog RL / Grant / PoP nonce / epoch API | closed（x00d2 / x00b5 / x00d4） | **確認**（`scene/save` の RL 抜けは別件の新規） |
| P2-3 GLB 寸法先読み | closed（x00d1） | **確認** |
| P2-4 KeyStale 無停止切替 | closed（x00d5） | **確認** |
| P2-5 docs / release 残骸除去 | closed（x00d1） | **partial**。README L117 と `smoke-rapier-host.sh` が残る |
| P2-6 assets CI ボリューム分離 | closed（x00d1） | **partial**。分離は確認、当日初回は別原因で失敗 |
| P2-7 relay-agent ureq | closed（x00d3） | **確認** |
| P2-8 Rooms / Fallback 分割 | closed（x00d6） | **確認** |

計画書の 16 項目のうち、コード上で計画どおり閉じていたのは 12 件、残件ありが 4 件。残件はいずれも「計画した範囲は実装したが、その周辺が残っている」型で、計画書の記述そのものが虚偽という意味ではない。

---

## 当日の重大度上位マイナス（トップ 5）

1. **既定物理経路の LBVH 構築が O(n²) で、毎サブステップ（60Hz × 4）作り直される** `-2`（新規・shared-sim）
2. **assets-server に HTTP レート制限が一つも無く、VCS push（12 MiB）・negotiate・SpawnTicket 発行が素通し** `-2`（新規・assets）
3. **新しいワイヤ経路（cell / lod / edit / sim_state）と SpawnTicket が E2E に入っていない。WAN E2E は既定 client がもう購読しない Room frame を検査している** `-1`（新規・テスト）
4. **既定にした cell 配信に、soft-put 失敗・remove 取りこぼしからの回復手段が無い** `-1`（新規・client/app）
5. **kick が鍵ローテだけで拒否リストが無く、public 部屋では追い出した相手が新鍵を取り直せる** `-1`（部分解消・world）

次点: 既定経路の毎 tick clone 3 回と一時確保（-1 継続）、`scene/save` だけ RL 対象外（-1）、overlay revision 上限で部屋が詰む（-1）。

---

## ローカル CI / テスト実行結果（2026-11-18・Windows 11 ネイティブ + Git Bash + Docker Desktop）

コマンド: `./.scripts/ci.sh all`（Git Bash）→ 失敗ステップのみ `./.scripts/ci.sh assets` で再実行

| ステップ | 結果 | 備考 |
|:---|:---|:---|
| client `cargo fmt --check` / `clippy -D warnings` / `cargo deny` / build | PASS | |
| client `cargo test --workspace` | PASS | 708 tests（前回 447） |
| relay-agent fmt / clippy / deny / build / test | PASS | 32 tests |
| protocol `cargo test` | PASS | 124 tests（前回 73） |
| protoc schema compile | PASS | `sim_state.proto` / `edit.proto` 含む |
| auth（Docker：hex.audit / format / compile -Werror / credo --strict / test） | PASS | 134 tests, 0 failures |
| world（Docker：同上） | PASS | 232 tests, 0 failures（前回 162） |
| assets（Docker：同上） | **FAIL（初回）→ 再実行 PASS** | 初回は `mix compile --warnings-as-errors` が `redefining module Assets.TestPlaceReadHelpers / Assets.DataCase / AssetsWeb.ConnCase / Assets.CasTmp / Assets.TestAuthHelpers (current version loaded from _build/test/…)` で失敗。単独再実行で 240 tests, 0 failures（前回 191）。CI 専用ボリュームが持ち越した古い `_build/test` による**状態依存の初回失敗**と判定（weaknesses -1） |
| shared/alchemy_*（Docker） | PASS | auth_client 13 / client_ip 5 / rate_limit 7 |
| prod compose config | PASS | |
| **RESULT（初回）** | **FAILED（assets のみ）** | 再実行で全緑相当 |

実行しなかったもの（環境制約）:

- `make ci-e2e`（ヘルメティック Compose E2E）と WAN host E2E：Linux ランナー前提の長時間ジョブのため未実行。静的にスクリプトとジョブ定義を読み、新経路が含まれていないことを確認した（weaknesses -1）。
- `make ci-fuzz`、`shared-sim-android-check`（NDK）、`forbid-unsafe-roots`：GitHub Actions 側のジョブ。定義のみ確認。
- XR 実機（OpenXR ランタイム）での手動確認。

---

## git の状態

- HEAD: `57ca452`（Merge PR #247）
- 前回 `8833821` から 135 コミット。主な流れ:
  - x009d〜x00a2：HostDelegation（canonical owner ∪ editor・session overlay・eligible takeover）、親 client 資格情報 broker（AUTH_PUSH）、kick / relation → epoch、handover 二段階、SceneBudget
  - x00a3〜x00ad：MVCC Phase 1〜3（page / arena / CoW / EntityDirectory・安全 SIMD・不等式接触・publish 基盤・jitter・手元予測・`interp.wgsl` 足場・host sim backend）
  - x00ae〜x00b9：dirty delta コーデック・PROTOCOL_PIN 0.24・`sim_state`・30Hz 切替・edit レーン（Spawn / Despawn / Reparent / SetField / Save）・子 Grant 必須・SpawnTicket・overlay → canonical Save
  - x00ba〜x00c7：Spatial Lens、Spatial Interest（cell frame → 既定化 → VR 配信契約 → avatar sync LOD）と一時停止メモ
  - x00c8〜x00cd：Spatial VCS（object コーデック・content addressed・Origin Cert v2・RepoRead・push・HTTP store / `client/vcs-store`）
  - x00ce〜x00d6：Grab ラジアル Drive 保存、OS vault、Desktop 分割、P2 衛生、catalog RL / epoch API、relay-agent ureq、PoP nonce、KeyStale 無停止、Rooms / Fallback 分割
- 差分規模: client/app +14.8k / -4.4k、client/host-server +7.7k / -0.4k、client/shared-sim +3.8k、client/vcs-store +1.1k（新規）、protocol +3.3k、world-server +7.5k / -2.3k、assets-server +6.0k / -0.8k、auth-server +0.16k、relay-agent +0.45k
- 新規依存（いずれも Cargo.toml に人の承認日つき）: `wide`（2026-11-14）、`keyring`・`ureq`（2026-11-17）。`rapier3d` は削除
- 作業ツリー: 前回評価文書のアーカイブ移動（`evaluation/*-2026-11-12.md` → `archive/2026-11-12/` 等）のみ未コミット。本評価では評価文書以外を変更していない。

---

## 技術評価層の所見（ブロック別講評）

### auth-server（+27 / 0）

変更は relation_changed の S2S 通知のみ。本番で通知先未設定なら起動しない fail-closed も入っている。Refresh ファミリー回転・多軸 rate limit・列挙対策は引き続き最強部品。

### assets-server（+41 / -2）

Spatial VCS の push は、git の receive-pack を「サーバーが全 object を Rust で正規形に戻して検証し、作者と Grant を確かめ、fast-forward を CAS で 1 TX」という Alchemy の原則で書き直した、筋の良い実装。Origin Cert v2 はオフライン検証まで用意し、連合の下地になった。ただし**入口の流量制御がゼロ**で、重い経路が増えたぶん弱点が目立つようになった（前回の自分の記述「assets も `alchemy_rate_limit` を共有」は誤りで、ここで訂正した）。

### world-server（+70 / -3）

最も伸びたブロック。HostDelegation は Resonite と同じ線引き（実行権と保存権の分離）を world の権限表として明示し、packed revision の CAS で原子性も担保した。SpawnTicket は「world が assets の DB を覗かずに資産の権利を確かめる」サービス間の権限証票として、このプロジェクトの層分離の原則をよく体現している。`Rooms` 分割と `FallbackController` で、前回指摘した「権限表の見落としが起きやすい構造」も解消した。残るのは kick の拒否リスト、`scene/save` の RL 抜け、overlay revision の時限式上限という周辺の 3 点。

### protocol（+48 / 0）

`vcs_object` の決定的 CBOR は、git の object 形式よりも厳密（非正規ヘッド・未知キー・末尾ゴミを拒否）。新しいワイヤ（`sim_state` / `edit`）も decode 上限と AEAD 封筒上限を先に課す形で入った。引き続き最も完成度が高い層。

### client/shared-sim（+11 / -3）

MVCC page・CoW・確保上限テスト・ビット一致の SIMD・不等式接触と、部品の質は高い。問題は**本番で走っている既定経路**が、その部品の恩恵を受けていないこと。LBVH の AABB 引き当てが線形探索で O(n²) になっており、radix sort まで入れた努力を打ち消している。確保ゼロを証明した経路（MVCC）と実際に走る経路（EpochXpbd）が別、という構図も前回から変わらない。

### client/host-server（+60 / -1）

edit レーンは「AEAD を開く前に封筒サイズ」「物理適用に成功してから Commit」「失敗しても tip を巻き戻さない」と、受理と反映のずれを外に出さない設計で、同種 OSS より一段厳格。Spatial Interest の host 側も、本線（Room frame）は fail-closed、補助（cell / lod）は soft-fail と失敗方針を分けている。ただし `sim.rs` と `network_worker.rs` がそれぞれ 3000 行前後に育ち、x0099 で解いた一枚岩が別の場所に戻りつつある。

### client/app（+67 / -2）

資格情報まわり（AUTH_PUSH broker・OS vault・KeyStale 無停止切替）は、OAuth の推奨（refresh は最も信頼できるプロセスだけ）をプロセス境界でそのまま実装した。手元予測と jitter クロックで体感の土台もできた。弱点は、既定にした cell 配信の回復手段の欠如（非保証欄には正直に書いてある）と、`dash.rs` の肥大。

### relay-server / relay-agent / edge（+26 / 0）

relay-agent の `curl` を外し、ACL は新キーを明示列挙して `frame/**` に広げなかった。当日マイナスなし。

### 横断

- **テスト**: client 708・world 232・assets 240・auth 134・protocol 124・relay 32。world は前回の見逃しを踏まえ、権限の否定系テストを厚くした。一方、新しいワイヤ経路の E2E が無いことは、今回もっとも「見逃しの温床」になりやすい穴。assets のローカル CI は原因を変えて初回失敗が再発した。
- **可観測性**: 前回水準を維持。
- **エラーハンドリング**: `FallbackController` の表と edit レーンの reject / 回復で一段整理された。
- **保守性**: 新しいモジュールも、弱点を含めて冒頭の非保証欄に書く運用が守られている。今回の弱点の多くは、その欄を読むだけで見つかった。依存追加に承認日を残す運用も良い。
- **DX**: README が必ず失敗する smoke を案内している。
- **セキュリティ**: 「誰に何の権限を渡すか」の層（HostDelegation・SpawnTicket・eligible）と「資格情報をどう運ぶか」の層（AUTH_PUSH・OS vault・PoP nonce）が揃った。`forbid(unsafe_code)` も CI のゲートになった。
- **全体設計**: 自己改善サイクルは前回の P0 2 件をどちらも権限モデルごと閉じた。ただし trust-boundary.md に edit / sim_state / SpawnTicket / VCS の行が無く、横断の正本への反映が遅れた。

---

## 同種 OSS との比較

| 軸 | 比較対象 | 所見 |
|:---|:---|:---|
| 権限モデル | Resonite / VRChat のインスタンス権限 | 実行権（host）と保存権（owner / editor）の分離は Resonite と同じ線に並んだ。VRChat のインスタンス ban に当たる拒否リストは未 |
| 部屋機密 | MLS / Matrix E2EE | 関係解除での epoch 前進は MLS の Remove に近い効果。ただし Matrix の ban に当たる「再入室拒否」が無いので、public 部屋では Remove 相当が成立しない |
| 物理 | Rapier（parry の BVH）/ avian | MVCC・決定論 SIMD・不等式接触は良い。既定経路の BVH は毎サブステップ全再構築かつ O(n²) で、parry の refit / 増分更新に明確に劣る |
| 配信 | Unreal Replication Graph / VRChat | cell / lod の AOI 配信は「全員に全員」より先を行く。Replication Graph の定期的な関連性再評価やキーフレームに当たる回復手段が無い |
| VCS | git（receive-pack / fsck / 署名付きコミット） | 決定的 object・サーバー再検証・fast-forward・作者証明 v2 は git より厳格。clone / fetch / 鍵ローテ後の過去証明の検証は未 |
| 資格情報 | OAuth 2.0 BCP / Steam クライアント | refresh を親だけが持ち OS ストアに置く形は BCP どおり |
| 流量制御 | Phoenix + Hammer 等の一般構成 | auth / world は操作別・主体別で一般水準以上。assets だけ無い |

---

## 次の一手（第1評価者の推奨順）

1. **LBVH の AABB 引き当てを添字にして O(n) に、サブステップ 2 回目以降は refit に**。同じ PR で `EpochXpbd` 経路にカウンタアロケータのテストと n = 4096 の時間計測を入れ、SceneBudget と物理予算を同じ数字で縛る。修正量は小さく、効果は最大。
2. **assets に `shared/alchemy_rate_limit` を入れ、push は 1 ユーザー同時 1 本に**。同じ流れで world の `scene/save` を `:host_scene` バケットへ。
3. **新ワイヤ経路の E2E**: e2e-probe に cell 購読と封緘 frame 受信、否定系（guest の cell put 不達・guest の `sim_state` 購読拒否・SpawnTicket の二重使用 403）を足す。ACL のキー一覧と E2E 検査項目を突き合わせるスクリプトも。
4. **cell 配信の回復（占有セルの低頻度キーフレーム or セル連番）と、kick の拒否リスト（deny + rotate を 1 CAS）**。
5. 衛生: trust-boundary.md に 4 行追加、`sim.rs` / `network_worker.rs` / `dash.rs` の分割、`smoke-rapier-host.sh` の削除、CI ボリュームの匿名化、overlay revision を canonical 更新時に 0 へ戻す。

---

## 評価の前提

- 旧 Elixir 権威実行前提は使っていない。
- 目標のみの機能（verifier 統計・billing / market・Dual-AST Kernel / OSC・Wake-on-Join・Spatial VCS の clone / branch 等）は減点せず提案または対象外とした。非保証欄に明記された未実装でも、**既定の本線になっている経路の性質**（cell 配信の回復・既定物理経路の確保）は減点対象にした。
- 第2評価者の文書は読んでいない。改善計画の「closed」注記は参照のみで、判定はすべて当日コードの再読に基づく。
- 前回の自分の強み（auth 項）にあった「`alchemy_rate_limit` を assets でも共有」は事実誤認だったため訂正し、今回 assets の -2 として計上した。前回の総合点は遡って修正していない。
- assets の初回失敗は再実行で消えたが、原因（CI ボリュームの持ち越し）は構造的なので、フレークではなく「状態依存の初回失敗」として -1 を残した。
