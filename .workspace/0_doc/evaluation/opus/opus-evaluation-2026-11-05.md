# Alchemy — 総合評価レポート（第1評価者 Claude Opus 5.5 / 2026-11-05）

> 評価対象: `main` HEAD `c491016a49eab6496a541178ea70df1952d5bc1e`（Merge PR #142 `feat/x007b-scene-despawn`）
> 前回: `d2d9f44`（2026-10-30）→ 今回まで **114 コミット**
> 詳細: [weaknesses](opus-specific-weaknesses-2026-11-05.md) · [strengths](opus-specific-strengths-2026-11-05.md) · [proposals](opus-specific-proposals-2026-11-05.md)
> 回帰の起点: [まとめ 2026-10-30](../archive/2026-10-30/specific-weaknesses-2026-10-30.md) · [自系統 2026-10-30](archive/2026-10-30/opus-specific-weaknesses-2026-10-30.md)
> 第2評価者（`gpt/`）の文書は当日・アーカイブとも参照していない。全判定は当日コードの再読に基づく。

## 総合点

| 項目 | 点 |
|:---|---:|
| プラス合計 | **+331** |
| マイナス合計 | **-23** |
| **正味** | **+308** |

> 前回は +262 / -36 = +226。プラスは前回の長所を当日コードで再確認したうえで、x0066〜x007b の新規分を積んだ（+69）。マイナスは前回 P0（relay / WAN / CAS）がすべてコード上で閉じたため大きく減ったが、**新しく入った inventory 制御面と Room 鍵 AEAD の周辺で、-3 と -2 の新規欠陥を 1 件ずつ見つけた**。

---

## 観点別スコア

| ブロック | 観点 | + | - | 正味 | 前回正味 |
|:---|:---|---:|---:|---:|---:|
| 運営側コア | auth-server | +25 | 0 | **+25** | +22 |
| 運営側コア | assets-server | +29 | -3 | **+26** | +18 |
| 運営側コア | world-server | +34 | -8 | **+26** | +19 |
| 共通契約 | protocol/ | +34 | -1 | **+33** | +28 |
| ホスト端末 | client/host-server | +43 | -3 | **+40** | +35 |
| ホスト端末 | client/app | +43 | -2 | **+41** | +32 |
| ホスト端末 | launcher/ | +7 | 0 | **+7** | +3 |
| 転送・エッジ | relay-server / edge / relay-agent | +21 | -1 | **+20** | +8 |
| 横断 | テスト戦略 | +26 | -2 | **+24** | +15 |
| 横断 | 可観測性 | +9 | 0 | **+9** | +4 |
| 横断 | エラーハンドリング | +6 | 0 | **+6** | +4 |
| 横断 | 変更容易性・保守性 | +13 | 0 | **+13** | +10 |
| 横断 | 開発者体験（DX） | +9 | -1 | **+8** | +7 |
| 横断 | セキュリティ・配布可能性 | +13 | 0 | **+13** | +9 |
| 横断 | プロジェクト全体設計 | +19 | -2 | **+17** | +12 |
| | **合計** | **+331** | **-23** | **+308** | **+226** |

Optional Profile（billing / market-server）はディレクトリ未設置のため評価対象外。

---

## 一言でいうと

前回「アプリ暗号契約は閉じたが transport 門番（relay）と CAS access 配置が残る」と書いた P0 5 件は、**1 週間で全部コードで閉じた**。しかも閉じ方が良い。部屋隔離は zenohd ACL の限界を認めたうえで Room 鍵 AEAD（XChaCha20Poly1305・AAD に room_id と kind）という根本策を取り、保証表の「`wire_cap` で隔離」という過大主張を自分で取り下げた。mTLS は world が CSR を署名する正経路に切り替え、NotAfter を Token TTL に揃えた。CAS は access カラム自体を消して権利を Grant / PlaceRead に移した。そしてそれらを **WAN 否定系 E2E** と **GUI と同じゲート関数を通る headless probe** で CI に載せた。個人プロジェクトの改善速度としては異例。

一方で、新しく機密性の正本になった **Room 鍵** の運用設計が追いついていない。鍵は部屋作成時に 1 度だけ作られ、公開範囲を狭めてもローテーションされない。さらに inventory 制御面は **host Room Token を bearer のまま、その共有鍵で包んで Zenoh に流す**設計で、world の host 専用 API は `cnf.spk` の所持証明を見ないため、部屋メンバーが自分の部屋を作って host 証明書を得れば、他人の部屋の host を乗っ取れる連鎖が成立する（現在の release GUI は guest Token しか持たないので潜在欠陥）。前回まで積み上げた「Untrusted host を署名で縛る」設計の、最後の 1 ピース（Token の運び方と PoP）が欠けている。

---

## 前回からの変化（回帰判定 A）

### まとめ 2026-10-30 のマイナス（12 行）

| # | 前回項目（点） | 判定 | 根拠 |
|:---:|:---|:---|:---|
| 1 | WAN ACL wildcard で `wire_cap` 部屋隔離が不成立（-3） | **closed** | Room 鍵 AEAD `protocol/src/wire_aead.rs` L92-140、AAD = `alchemy/aead/v1\|room_id\|kind`。trust-boundary.md L37 で `wire_cap` 秘匿を非保証へ。WAN 否定系 `e2e-compose.yml` L95-123。※鍵ローテ欠如は新規 -2 として別立て |
| 2 | 300 秒証明書の実 NotAfter が 24h（-3） | **closed** | `relay-agent/src/lib.rs` L493-556（`-not_after now+300`）、読み戻しテスト L1224-1312。world 側 `mtls_issuer.ex` L215-246 |
| 3 | Allocation の JWKS hard / body / thread / jti（-3） | **closed** | `JWKS_HARD_TTL` L34, L349-363, L423-434／`MAX_BODY_BYTES` L605-640／`MAX_IN_FLIGHT`・`MAX_CONNECTIONS` RAII L156-179, L293-305／jti 予約 L465-481, L677-690 |
| 4 | relay が秘密鍵を平文 HTTP で返す・手順書 0.0.0.0（-2） | **partial** | 正経路は world の CSR 署名（鍵はネットワークに出ない・`mtls_issuer.ex` L28-33）。relay-agent 互換経路は今も `key_pem` を返す（`lib.rs` L507-521, L700-708）→ -1 で残置 |
| 5 | CAS access 先着固定（-3） | **closed** | `CasObject` から access 削除、GET は Grant or PlaceRead（`blobs.ex` L12-16, L459-470）。mime 先着は新規 -1 |
| 6 | relay-agent が CI・deny・テスト外（-2） | **closed** | `rust-ci.yml` L129-153、`ci.sh` L58-72・L188、当日 16 tests PASS |
| 7 | adversarial E2E が実 client trust path を通らない（-2） | **closed** | `client/trust` を GUI（`bus/manifest.rs` L20）と `trust-probe` が共有、WAN 否定系ジョブ |
| 8 | world rate limit / Room 上限なし（-2） | **closed** | `router.ex` L12-22 + `plugs/rate_limit.ex`、`rooms.ex` L579-604（既定 32）。Room Token 系ルートの無制限は新規 -1 |
| 9 | 相関 ID / auth・world・relay メトリクス不足（-1） | **closed** | x007a：world `[:request_id, :jti]`、auth `[:request_id, :access_jti]`、relay / host の `room_jti` ログ、6 プロセスの loopback metrics |
| 10 | launcher host restart 後の IPC stale（-1） | **closed** | `launcher/src/discovery.rs`、`launcher.md` L16-27 |
| 11 | README / client.md / vision 鮮度（-1） | **closed** | README 追従。`client.md` / wan / world / vision は **x008a** |
| 12 | 軽微群（-3） | **partial** | closed: auth Keys PT・seal CLI・reserved 回収・room_token 分割・client main 分割・tos UI・DemoScene ベクタ・prod 0.0.0.0・cargo deny 範囲・mTLS temp・腐った smoke（x0089）。open: host `main()`・USER_ROOT 天井。partial: BLAKE3（Port 化したが直列） |

**集計（まとめ 12 行）: closed 10 / partial 2 / open 0 / superseded 0**

### 自系統 2026-10-30 のマイナス（30 件）

| # | 前回項目 | 判定 | 根拠（当日再読） |
|:---:|:---|:---|:---|
| 1 | auth Keys GenServer 直列化 | closed | `keys.ex` L9-13, L62, L83 |
| 2 | BLAKE3 を `b3sum` CLI + 一時ファイル | partial | Port 化（`blake3.ex`）。単一 GenServer 直列で -1 残置 |
| 3 | `reserved_bytes` 回収なし | closed | `quota/reserved_reaper.ex`、`blobs.ex` L249-273 |
| 4 | world rate limit / Room 上限なし | closed | 上記 #8 |
| 5 | 共有 JWKS 取得失敗時クールダウンなし | open | `jwks.ex` L149-152, L187（変化なし） |
| 6 | ManifestSeal が毎回一時ファイル + CLI | partial | Port 化（`manifest_seal_hash/worker.ex`）。`/health` 非反映で -1 |
| 7 | `USER_ROOT_SLOTS_MAX = 4` | **closed**（x009b） | 帯 `0x1000_0000..` · DoS 天井 32 · 幅は Manifest |
| 8 | host `main()` 一枚岩 | open | `main.rs` L790〜（約 640 行に拡大） |
| 9 | `room_token.rs` 2000 行超 | closed | `room_token/` 7 ファイル分割（x0076） |
| 10 | bind 全体バケット枯渇 | closed | x0077 分割 → **x009c** session_pub 起点 + soft-global 返却 |
| 11 | 配信 tick ごとの `Vec` 確保 | open | `main.rs` L1286-1301 |
| 12 | `tos_agreed: true` 固定 | closed | `auth.rs` L140-155、`desktop.rs` L1288-1289 |
| 13 | client `main.rs` 1899 行 | closed | 274 行。代わりに `dash.rs` / `desktop.rs` が肥大（新規 -1） |
| 14 | launcher IPC stale | closed | x0072 |
| 15 | WAN ACL wildcard（-3） | closed | 上記 #1 |
| 16 | relay 平文鍵 / 0.0.0.0（-2） | partial | 上記 #4 |
| 17 | 証明書 NotAfter 1 日 | closed | 上記 #2 |
| 18 | prod compose 0.0.0.0 公開 | closed | 3 ファイルとも `${PUBLISH_ADDR:-127.0.0.1}` |
| 19 | relay-agent CI 外・テスト 0 | closed | 上記 #6 |
| 20 | adversarial E2E が client discard / WAN を通らない | closed | 上記 #7 |
| 21 | 旧 smoke が腐っている | closed | x0089: banner-grep smoke 削除・`ci-e2e` へ吸収 |
| 22 | 相関 ID / auth・world メトリクスなし | closed | 上記 #9 |
| 23 | relay-agent の `expect` / `unwrap` | partial | 本番経路は大半 `map_err`。`jwks_age()` の `expect` 残（L344-346） |
| 24 | DemoScene 手動同期 | closed | `protocol/testdata/demo_scene_seal.json` + 両言語テスト |
| 25 | README / client.md 古い | closed | 上記 #11（**x008a**） |
| 26 | `make ci` が relay・shared・fuzz を覆わない | partial | relay + shared（x0089）は追加。fuzz は未（`ci-fuzz` 別入口） |
| 27 | cargo deny が client のみ | closed | launcher / relay-agent も deny（`rust-ci.yml` L120-123, L144-147） |
| 28 | mTLS 鍵 temp 権限・掃除 | closed | `allocate.rs` L280-345（icacls / 0600 / sweep） |
| 29 | 保証表が `wire_cap` 隔離を過大主張 | closed | trust-boundary / relay-server.md / `config.wan.json5` / `relay-server-wan.md` を Room 鍵 AEAD（x0069）へ。過大主張は **x008a** で閉じた |
| 30 | `vision.md` 見出しのみ | closed | **x008a** で最終目標の短文 |

**集計（自系統 30 件）: closed 20 / partial 6 / open 4 / superseded 0**

### improvement-plan で done とされた x0074〜x0079 のコード確認

| ID | 計画上の状態 | 当日確認 |
|:---|:---|:---|
| x0074 reserved reaper + orphan CAS GC | done | **確認**。reaper と GC が別 GenServer、GC は quarantine → 再確認 → discard/restore（`orphan_gc.ex` L1-10） |
| x0075 BLAKE3 / seal-hash 常駐 Port | done | **確認**。ただし両方とも単一 GenServer 直列（-1 残置） |
| x0076 host room_token / client main 分割 | done | **確認**（スコープ内）。host `main()` 自体は対象外で肥大継続 |
| x0077 bind 送信元分割 | done | **確認**。自己申告キー残は **x009c** で closed |
| x0078 mTLS 鍵 temp ACL / 掃除 | done | **確認**。Windows DACL を実検証するテストあり |
| x0079 DemoScene 共有テストベクタ | done | **確認**。Elixir / Rust が同じ JSON を照合 |

計画書 P0-1〜P0-5・P1-1〜P1-5 も x0066〜x0073 でコード上閉じていることを確認した（P0-4 のみ relay-agent 互換経路が残るため partial）。

---

## 当日の重大度上位マイナス（トップ 5）

1. **host Room Token が bearer のまま Zenoh 制御面（spawn / despawn）に載り、world の host API は PoP を見ない → host 乗っ取り連鎖** `-3`（新規）
2. **Room 鍵が作成時に 1 度だけ生成され、公開範囲変更・kick でローテーションされない** `-2`（新規）
3. **Room Token 系 world ルートに rate limit が無く、seal hash は単一 Port に直列化** `-1`（新規）
4. **relay-agent 互換経路が今も鍵をサーバー生成して `key_pem` を返す** `-1`（部分解消）
5. **assets の `mix test` が既定並列でフレーク（当日初回 54 failures）** `-1`（新規）

---

## ローカル CI / テスト実行結果（2026-11-05・Windows 11 ネイティブ + Git Bash + Docker Desktop）

コマンド: `./.scripts/ci.sh all`（Git Bash）

| ステップ | 結果 | 備考 |
|:---|:---|:---|
| client `cargo fmt --check` / `clippy -D warnings` / `cargo deny` / build | PASS | |
| client `cargo test --workspace` | PASS | 275 tests |
| launcher fmt / clippy / deny / build / test | PASS | 8 tests |
| relay-agent fmt / clippy / deny / build / test | PASS | 16 tests（NotAfter 読み戻し含む） |
| protocol `cargo test` | PASS | 67 tests |
| protoc schema compile | PASS | |
| auth（Docker：hex.audit / format / compile -Werror / credo --strict / test） | PASS | 130 tests, 0 failures |
| world（Docker：同上） | PASS | 116 tests, 0 failures（ログに `could not lookup Ecto repo World.Repo` のノイズあり・失敗ではない） |
| assets（Docker：同上） | **FAIL（初回）** | 168 tests, **54 failures**（`Database busy` / `rm_rf priv/cas_test`）。同コンテナで `mix test --max-cases 1` → 0 failures、既定設定で再実行 → 0 failures。**フレーク**と判定 |
| prod compose config（x003b） | PASS | |
| **RESULT** | **FAILED（assets のフレークのみ）** | 再実行で全緑相当 |

実行しなかったもの（環境制約）:

- `make ci-e2e`（hermetic Compose E2E）と `wan-negative-e2e`：Linux ランナー前提の長時間ジョブのため未実行。GitHub Actions の結果は `gh` 未認証のため取得できなかった。
- `make ci-fuzz`：nightly + Linux / macOS 前提。
- `shared/alchemy_*` 3 パッケージの `mix test`：`ci.sh` に含まれないため未実行。

---

## git の状態

- HEAD: `c491016a49eab6496a541178ea70df1952d5bc1e`（2026-11-05, Merge PR #142）
- 前回評価コミット `d2d9f44` から 114 コミット。主な流れ:
  - x0066〜x0068：relay-server リネーム、world 発行の短命 mTLS（CSR）、zenohd ACL の非対称化（x0067）、relay-agent NotAfter / 上限 / CI
  - x0069：Room 鍵 AEAD（PROTOCOL_PIN `0.18.0-x0069`）
  - x0054〜x0065 / x006b〜x006f：inventory ladder Wave 1〜4、PlaceRead、BlobGrant 案 B、take-out / put-back、CAS access → Grant 移設
  - x0053：Desktop / XR hot switch 実装
  - x0070〜x0073：headless trust + WAN 否定系 E2E、world rate limit + Room 上限、launcher discovery、P3 衛生
  - x0074〜x007b：reaper / orphan GC、BLAKE3 Port、保証単位分割、bind 送信元分割、mTLS temp、DemoScene ベクタ、metrics + jti 相関、scene despawn
  - 4 階層マルチライセンス分離（#119）
- 作業ツリー: 評価文書のアーカイブ移動（`evaluation/*-2026-10-30.md` → `archive/2026-10-30/`）のみ未コミット。

---

## 技術評価層の所見（要約）

### auth-server（+25 / 0）

前回唯一の負債（Keys の直列化）が `:persistent_term` + fail-closed で解消。新規の欠陥は見つからなかった。Refresh ファミリー回転・多軸 rate limit・列挙対策は引き続き OSS 認証基盤（Ory Kratos / Keycloak の最小構成）と比べても遜色ない。

### assets-server（+29 / -3）

access の先着固定を「カラムを消す」ことで解き、GET を常時 Grant / PlaceRead にした。reaper / orphan GC の TOCTOU 設計は水準以上。残るのは BLAKE3 Port の直列化、mime 先着、Origin Cert が内容ハッシュを署名しない点。

### world-server（+34 / -8）

CSR 署名の mTLS・PlaceRead の seal 束縛・rate limit と、門番としての厚みが大きく増した。その一方で、今回最大の欠陥（host Token の PoP 欠如 → 乗っ取り連鎖）と Room 鍵の非ローテーションはどちらも world が根本原因。**機密性と権限の正本が world に集まった分、world の鍵・Token のライフサイクル設計が次の焦点**。

### protocol（+34 / -1）

Room 鍵 AEAD が加わり、ワイヤ契約は「decode 上限 → AEAD → 署名 → gen/rev ゲート」の多層を protocol の純関数で共有する形になった。前回同様、このプロジェクトで最も完成度が高い層。

### client/host-server（+43 / -3）・client/app（+43 / -2）

`room_token.rs` と client `main.rs` の分割、`client/trust` クレートの切り出しで保守性が上がった。新しい肥大（host `main()`、`dash.rs` / `desktop.rs`）と、take-out UI が release で host Token を持てない不整合が残る。

### launcher（+7 / 0）

discovery で前回の唯一の負債を閉じた。トレイ UI・ディープリンクは目標のまま（減点なし）。

### relay-server / relay-agent / edge（+21 / -1）

前回 -7 の最弱ブロックが +20 へ。zenohd ACL を実機 trace で詰め、relay-agent を bounded にし、world に正経路を移した。互換経路の `key_pem` だけが残る。

---

## 次の一手（第1評価者の推奨順）

1. **制御面を session 鍵署名に切り替え、world host API に PoP を入れる**（トップ 1 の解消。新規依存なし）
2. **Room 鍵 epoch 化**（visibility 縮小・kick でローテ。保証表に `room_key` を載せる）
3. **Room Token 系ルートの rate limit + Port プール + `/ready`**
4. **relay-agent を CSR 専用化 or 退役**
5. **assets テストの reaper / GC を test で起動しない・CAS パスを tmp_dir 化**、腐った smoke の削除、`ci.sh` に shared パッケージ追加
