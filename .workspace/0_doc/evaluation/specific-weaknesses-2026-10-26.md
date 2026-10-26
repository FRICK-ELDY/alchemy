# Alchemy — マイナス点統合一覧（まとめ / 2026-10-26）

> 第1評価者（Claude Opus 5.5 / 正味 +24）と第2評価者（GPT-5.6 Sol / 正味 -1）の独立評価を統合。
> **採用**はまとめの判断。点数は採用スコア（合意時は近い方、相違時は根拠の強い方を採用し注記）。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 改善余地あり。動作はするが設計・品質上の軽微な問題 |
| -2 | 重要な機能・設計の欠如。放置すると将来の拡張を阻害する |
| -3 | 設計上の明確な欠陥。バグ・クラッシュ・性能劣化を引き起こしうる |
| -4 | プロジェクトの価値命題を損なう重大な欠如。説明責任が果たせない |
| -5 | プロジェクトの根幹を揺るがす致命的な欠陥。存在しないに等しい |

---

## 両者が合意した重大欠陥（優先）

### 転送・公開エッジ / セキュリティ

- **認証なし `host_control` で誰でも host を終了できる** `-5`（採用: GPT）
  > Zenoh に `shutdown` を put するだけで `process::exit(0)`。WAN が平文・無 ACL の現状と組み合わさると可用性攻撃が即時成立する。Opus は `-2`（スタブ明記を考慮）だが、コードが実際に subscribe・exit しているため GPT の `-5` を採用。
  > 対象ファイル: `client/host-server/src/host_control.rs`
  > 評価者: Opus `-2` / GPT `-5` → **採用 `-5`**

- **Tunnel 手順が dev ビルドを公網公開し、mailbox 経由でアカウント乗っ取り可能** `-4`（採用: Opus）
  > `docker compose up`（`MIX_ENV=dev`）を Tunnel 手順が指示。`/dev/mailbox` が認証なしで見え、forgot-password → mailbox → reset で任意アカウントを奪える。GPT は個別項目化しなかったが、同一の trust boundary 問題として採用。
  > 対象ファイル: `edge/cloudflared/README.md`, `auth-server/docker-compose.yml`, `auth-server/lib/auth_web/router.ex`
  > 評価者: Opus `-4` / GPT（未採点・WAN に包含）→ **採用 `-4`**

- **WAN Zenoh が平文 TCP＋IP allowlist のみ** `-4`（採用: GPT）
  > Frame / Presence / Manifest / control に transport identity も暗号もない。Opus は `-2`（allowlist を部分緩和）。公開 WAN リレー価値命題に対しては GPT の厳しさを採用。
  > 対象ファイル: `zenoh-router/config.wan.json5`
  > 評価者: Opus `-2` / GPT `-4` → **採用 `-4`**

- **liveliness Delete で他人を退室させられる** `-3`（採用: Opus）
  > Presence で公開された `sub` を使い、liveliness を宣言してすぐ取り下げればスロット解放。GPT は明示採点なしだが host 防御の穴として採用。
  > 対象ファイル: `client/host-server/src/main.rs`
  > 評価者: Opus `-3` / GPT（未採点）→ **採用 `-3`**

- **共有バス上の Room Token 盗聴で 300 秒なりすまし** `-3`（採用: Opus）
  > InputEvent に Bearer JWT を毎フレーム載せ、subscribe 可能な peer が再利用できる。
  > 対象ファイル: `protocol/proto/input_events.proto`, `client/host-server/src/main.rs`
  > 評価者: Opus `-3` / GPT（WAN・Frame 無検証と連鎖）→ **採用 `-3`**

### assets-server / world-server

- **CAS hash を知る認証ユーザーが他人 Blob を取得できる** `-3`（採用: 折衷）
  > GET は owner_sub を見ない。overview の「見る権利は全員 OK」と整合しうるが architecture に明記がなく、有料アセット前提では欠陥。GPT `-4`、Opus `-1`（文書化不足）。設計意図未文書化＋門番価値の毀損として `-3`。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`
  > 評価者: Opus `-1` / GPT `-4` → **採用 `-3`**

- **任意ユーザーが任意 room の host role を発行できる** `-3`（採用: 折衷）
  > request の role をそのまま署名し、存在しない room も ensure。現状 host は role で権限分岐していないが、門番価値命題と将来の verifier に直結。GPT `-4`、Opus `-2`。
  > 対象ファイル: `world-server/lib/world_web/controllers/room_controller.ex`
  > 評価者: Opus `-2` / GPT `-4` → **採用 `-3`**

- **Quota / CAS の並行整合性がない** `-2`（採用: 折衷）
  > read-check と increment が非原子。GPT `-3`、Opus `-1`。
  > 対象ファイル: `assets-server/lib/assets/blobs.ex`
  > 評価者: Opus `-1` / GPT `-3` → **採用 `-2`**

### client / host

- **既定シード資格情報の暗黙利用** `-3`（採用: 折衷）
  > env 未設定時 `alice` / `Password1`。GPT `-4`、Opus `-1`。公開・配布経路では fail-open として重い。
  > 対象ファイル: `client/host-server/src/room_token.rs`, `client/app/src/auth.rs`
  > 評価者: Opus `-1` / GPT `-4` → **採用 `-3`**

- **magnitude / rate / Grab 距離の制限なし** `-3`（採用: 合意）
  > `is_finite` のみ。物理発散・遠距離 Grab が可能。overview の「100% 遮断」は未達。
  > 対象ファイル: `client/host-server/src/finite.rs`, `client/host-server/src/sim.rs`
  > 評価者: Opus `-2` / GPT `-3` → **採用 `-3`**

- **client が Frame/Manifest/Presence を無検証適用** `-3`（採用: GPT 寄り）
  > publisher 署名なし、NaN/Inf 未検査、sequence 未使用。Opus は個別弱め、GPT `-4`。
  > 対象ファイル: `client/app/src/main.rs`
  > 評価者: Opus（部分）/ GPT `-4` → **採用 `-3`**

- **Room Token を更新せず入室 5 分で操作不能** `-3`（採用: Opus）
  > TTL 300 秒、再取得なし。デモ中に顕在化する機能バグ。GPT 未採点。
  > 対象ファイル: `client/app/src/main.rs`, `client/app/src/auth.rs`
  > 評価者: Opus `-3` / GPT（未採点）→ **採用 `-3`**

- **host の JWKS 更新が入力スレッド同期・失敗で全入力停止** `-3`（採用: Opus）
  > world ダウン後 60 秒で入力全滅、未知 kid で HTTP 発火。GPT 未採点。
  > 対象ファイル: `client/host-server/src/room_token.rs`
  > 評価者: Opus `-3` / GPT（未採点）→ **採用 `-3`**

---

## 両者が合意した中程度・横断欠陥

### テスト / DX / 依存

- **assets ローカル CI が `.env` で本番 R2 に接続し失敗** `-3`（合意）
  > `runtime.exs` が test でも CAS backend を上書き。当日双方とも 7 件中 1 件失敗を確認。
  > 対象ファイル: `assets-server/config/runtime.exs`, `.scripts/ci.sh`

- **サービス横断自動 E2E なし・smoke は vacuous success** `-2`（合意）

- **既知 HIGH advisory（hackney）を CI gate しない** `-2`（採用: 折衷）
  > GPT `-3`、Opus `-1`。S3 経路上の依存なので `-2`。
  > 対象ファイル: `assets-server/mix.lock`, `.github/workflows/`

### 契約・実装品質

- **room_id 検証が protocol と world で不一致** `-2`（合意）

- **PROTOCOL_PIN がワイヤに載らない** `-2`（採用: Opus）

- **RenderFrame に回転がない** `-2`（採用: Opus）

- **コンテンツ定義が host/client に直書き・複製（定義 vs 実行の逆転）** `-3`（採用: Opus）
  > GPT は entity 定数重複を `-2`。層間原則違反として Opus `-3` を採用。
  > 対象ファイル: `client/host-server/src/sim.rs`, `client/app/src/scene.rs`

- **JWKS/Verify が world と assets でコピー** `-2`（採用: Opus）

- **JSON Content-Type で空 Blob が黙って保存** `-2`（採用: Opus）

- **Mutex poison を空 Presence として継続** `-2`（合意）

- **UI スレッドの blocking HTTP** `-2`（合意寄り）

- **物理ループ内の同期 Zenoh publish** `-2`（採用: GPT）

- **delta 順序・欠落・再同期契約なし** `-3`（採用: GPT）

- **remote/box の UBO alias** `-2`（採用: GPT）

- **hash 失敗時 fixture で隠蔽** `-2`（採用: GPT）

- **Refresh 同時 rotation に排他なし** `-2`（合意）

- **Tunnel 背後で Client IP が潰れレート制限共有** `-2`（採用: Opus）

- **未知 kid の同期 JWKS 再取得にクールダウンなし（world/assets）** `-2`（採用: Opus）

- **XR 生 Vulkan と Desktop wgpu の二重経路 / undocumented unsafe** `-2`（採用: Opus）

---

## 軽微（合意・片方）

- protoc CI が全 schema を対象にしない `-1`
- world テスト薄い（6 件）・host Room Token 否定系不足 `-1`
- 500 応答に `inspect(reason)` `-1`
- Room 一覧の無制限増加 `-1`
- BLAKE3 を外部 CLI＋tmp `-1`
- Keys の GenServer 直列化 `-1`
- リポジトリ直下の誤コミット `-o` `-1`
- Rust CI にキャッシュなし `-1`
- 登録時 ToS 同意の自動送信 `-1`
- vision / launcher.md 空・文書ずれ `-1`
- fuzz/property 不在 `-1`
- 箱の毎 tick 全件 upsert `-1`
- main 巨大・publish 失敗で process 終了 `-1`

---

## 採用マイナス合計（概算）

合意・折衷を積み上げた採用マイナスはおおよそ **-70〜-75** 帯。第1評価者（-72）と第2評価者（-62）の間にあり、重大項目は GPT 寄りに厳しさを寄せた。
