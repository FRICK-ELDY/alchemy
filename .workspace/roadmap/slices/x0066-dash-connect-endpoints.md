# x0066 — Dash Connect タブ（エンドポイント設定）＋テスター env 廃止

> **ID:** `x0066` · **slug:** `dash-connect-endpoints` · **status:** `active` · **area:** `[client, system_ui, launcher, docs]` · **surface:** `[desktop]` · **depends_on:** `[x0018, x0032]`

目標: system_ui に **Connect タブ**を追加し、テスター／日常クライアントが端点を **UI で設定・永続**できるようにする。Windows テスター ZIP から **URL / Zenoh 接続先のための `env.local.cmd` 手編集を廃止**する。

### UI: 簡易設定 / 詳細設定

表示順（詳細・契約の見え方）:

```text
auth.example.tld
assets.example.tld
world.example.tld
├ relay.example.tld     ← 選択中（セッションに使う 1 本）
├ （追加候補…）
└ +
```

| モード | 入力 | Apply 時の展開（案） |
|:---|:---|:---|
| **簡易** | `example.tld`（1 欄） | 上表の 3 HTTP + world 配下に `relay.example.tld` を 1 本（スキーム／ポートは実装時既定。ローカルは別既定） |
| **詳細** | Auth / Assets / World + **world 配下の relay リスト**（`+` で追加、1 本を選択） | そのまま永続・適用 |

簡易 → 詳細へ切り替えたときは展開結果を流し込み、そこから個別修正できる。詳細が簡易展開と一致しないときは簡易欄を空または「カスタム」表示（実装時に選ぶ）。

| UI（詳細・上から） | 現行 env | 対象 | 親子 |
|:---|:---|:---|:---|
| Auth | `AUTH_URL` | auth-server | 独立 |
| Assets | `ASSETS_URL` | assets-server | 独立（world の子ではない） |
| World | `WORLD_URL` | world-server | 親 |
| └ Relay（1 本以上・選択 1） | `ZENOH_CONNECT`（＋ `RELAY_ALLOCATE_URL` は同ホスト既定 or 行内拡張） | zenoh-router / Relay Agent | **world の子** |

正本（実装時更新）: [client.md](../../0_doc/architecture/client.md) · [launcher.md](../../0_doc/architecture/launcher.md) · [windows-tester-zip.md](../../0_doc/manual/windows-tester-zip.md)

### 信頼チェーンと整合（破綻していないか）

```text
auth  ──(access JWT)──►  world  ──(Room Token + Allocation Token)──►  client/host
assets ◄──(Bearer + CAS)── client     │
                              └── JWKS ──► 選んだ Relay Agent ──► zenohd
```

| 判断 | 内容 |
|:---|:---|
| **破綻していない** | relay を **world の子**にするのは Allocation／JWKS 契約と一致。Auth→Assets→World の並びも「口座 → 物置 → 案内所＋回線」で説明可能 |
| **Assets は sibling** | 木の見た目で assets を world 配下にしない（CAS は world 発行物ではない）。並びだけ World の前 |
| **本 Slice の relay `+`** | 候補は **本数上限なし**（表示はスクロール）。**手動で選択中 1 本**だけ接続。リスト順＝将来の優先順（上から） |
| **後続（本 Slice 外）** | Ping 最良への自動接続、障害時の上から順フェイルオーバー、公認リレー屋の自動発見 |
| **運用上の注意（非破綻・制約）** | world 変更時は **relay 候補は残し、allocate のみ再導出**して即 Apply。host/client は **同じ選択中 relay** を共有。LAN/tcp は allocate なしでも open 可だが、誤った relay だと空振り |
| **assets 契約** | Connect Apply は `SharedState.assets_url` を更新する。起動時メッシュの remount はしない（再入室／再起動で新 URL） |

つながる条件の要約: **world が auth を信頼し、選んだ Relay Agent が同じ world の Allocation を検証できる**こと。auth と relay の直結セットではない。

### 連合まで見ると使いづらくないか

**木の形（world⊃relay）自体は連合でも正しい。** 使いづらくなるのは「連合のたびに Connect の 4 欄を手で書き換える」運用にしたとき。

overview 上の連合は主に次の分離になる。

| 層 | 誰が持つか | 例 |
|:---|:---|:---|
| **拠点（ホームノード）** | ユーザーの口座・個人 Vault の所属 | auth A / assets A（長く固定） |
| **セッション先** | 今入る部屋の門番・回線 | world B の Room/Allocation + B が指名する relay |
| **連合 S2S** | サーバ同士 | World 一覧 / JWKS / Market（**client の Connect ではない**） |
| **ディープリンク / node-info** | 後続 UX | `alchemy://` や一覧から **セッション用 world+relay を一時適用** |

だから Connect を **「毎回の行き先エディタ」**にすると破綻というより **連合 UX として重い**（他運営の部屋に行くたび Auth まで触りたくなる／触ってしまう）。

| 結論 | 内容 |
|:---|:---|
| **x0066 の位置づけ** | Connect = **拠点プロファイル**（テスター／単一運営スタックの正本）。Auth·Assets·（既定）World·（既定）relay |
| **連合入室（後続）** | 一覧・招待・ディープリンクが **セッション上書き**（world + 選択 relay。必要なら assets base）。拠点の auth は基本いじらない |
| **今の木を捨てない理由** | セッション先でも Allocation 契約は常に **その部屋の world⊃relay**。UI を「拠点木」と「セッション上書き」の二段にすれば再利用できる |
| **命名の罠** | 詳細の World 欄は「今いる空間」ではなく **既定の案内所 URL**。連合 UI では「ホーム world」と明示した方がよい（実装／文言は後続でも可） |

本 Slice は拠点手入力まで。連合一覧・S2S・node-info 自動適応はやらない（下「やらない」）。

## なぜ入れるか

| 現状 | 問題 |
|:---|:---|
| 端点は起動時 `env` のみ（client / host） | テスターは `env.testers.example.cmd` → `env.local.cmd` をメモ帳で編集する必要がある |
| Home は login／入室のみ | 接続先をアプリ内で変えられない |
| ZIP は auth/world/assets/zenohd 非同梱 | 遠隔スタック参加が「テキスト設定ファイル」前提になっている |

Connect タブで **拠点**端点を UI 入力しローカルに永続すれば、配布 ZIP は **ダブルクリック → Connect（拠点）→ Home login** の動線に寄せられる。連合の部屋渡りは後続のセッション上書きに任せる。

## 設計方針（層分け）

| 層 | 役割 | 知らないこと |
|:---|:---|:---|
| **system_ui（Connect Facet）** | 簡易 1 欄 / 詳細（Auth·Assets·World + world 配下 relay リスト·`+`·選択）。Home と同様 **フォームだけ** | HTTP / Zenoh open / Room Token / OpenXR / JWKS 検証 |
| **client app / surface** | 永続・展開・**選択中 relay 1 本**の適用。world 変更時は allocate 再導出（候補保持） | Facet レイアウトの詳細 |
| **host-server** | 同一端点契約（**同じ選択中 relay** 含む）を受け取る | system_ui |
| **launcher / Start.cmd** | テスター起動時に **URL 系 env を必須としない**。CI／運用者向け **任意 override** は残してよい | Dash タブ UI |
| **ensure-demo-coin** | 本 Slice では **資格情報付き seed を Start 必須から外す**か、運用者任意スクリプトへ退避（下表） | Connect フォーム |

`alchemy-node-info.json` / 連合一覧によるセッション上書き・リレー SLA 自動切替は **本 Slice 外**。本 Slice は **拠点の簡易ドメイン展開 + 詳細（world 配下 relay 手入力・選択 1 本）+ 永続**。

## 永続・優先順位（案）

| 優先（高い順） | ソース | 用途 |
|:---|:---|:---|
| 1 | プロセス環境変数（既存名） | CI / hermetic e2e / 開発者 override |
| 2 | ローカル設定ファイル（例: `%LOCALAPPDATA%/alchemy/connect.json` 相当。秘密を書かない） | テスター・日常 UI |
| 3 | コンパイル時既定（`127.0.0.1:4002` 等） | ローカル日常スタック |

保証: ファイルには **Auth / Assets / World URL・relay 候補配列・選択インデックス**のみ。`AUTH_PASSWORD` / トークンは **書かない**（Home / 既存 silent login のまま）。

## 適用タイミング（案）

| 操作 | 保証（案） |
|:---|:---|
| 未入室で Apply | probe（JWKS / Zenoh）→ SharedState → `connect.json` の順で確定。途中失敗はランタイムをロールバック。永続成功後のみ auth 変更時に session clear |
| 入室中に Apply | **拒否**（Leave してから） |
| 次回起動 | 永続値をロード（env があれば env 勝ち） |

host と client の端点が食い違うと Token / Zenoh が壊れるため、**launcher 子プロセスへ同じ値を渡す**か **両方が同じ設定ファイルを読む**ことを成功条件に含める。

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 本 Slice + 目次 | done |
| 1 | 端点設定型 + 永続 I/O + env 優先ロード（`alchemy-connect-config`） | done |
| 2 | Dash **Connect**（簡易 / 詳細・Auth→Assets→World→relay 木・`+`・選択・Apply） | done |
| 3 | client 実行経路を設定源に接続（login / assets / world / **選択中** zenoh-connect） | done |
| 4 | host + launcher 配線（同一設定）。テスター `Start.cmd` / README-testers / windows-tester-zip から **URL 手編集必須を削除** | done |
| 5 | ensure-demo-coin 必須資格情報の切り離し（任意）+ docs | done |

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Desktop Dash に Connect（**簡易 `example.tld`** / **詳細: Auth·Assets·World + world 配下 relay `+`**） | VR 仮想キーボード；`alchemy-node-info` 自動発見 |
| 簡易 Apply で auth/assets/world + relay 1 本の既定 FQDN に展開される | 任意サブドメイン規約のサーバ側強制（クライアント展開のみ） |
| relay 候補を件数上限なく持て、**手動選択 1 本**だけ接続に使う（表示はスクロール） | Ping 自動選択・障害時上からフェイルオーバー・同時マルチ接続（後続） |
| Apply（未入室）で端点が以降の接続に効く（失敗時ロールバック） | 入室中ホットスワップ；誤った relay でも空振りしないこと；Apply 後の起動メッシュ remount |
| 端点のローカル永続（秘密なし）と、env による CI override | クラウド同期・プロファイル複数・ディープリンク注入 |
| テスター ZIP: **URL / relay のために `env.local.cmd` を書かなくても** Connect UI で参加動線が成立 | auth/world/assets/zenohd の ZIP 同梱 |
| host と client が同じ端点契約（選択中 relay 含む）を共有できる | host handoff・複数 room |
| Home login（資格情報）は Connect と分離したまま | `env.local.cmd` 自体の完全削除（運用者任意 override は残してよい） |

## テスター配布（env 廃止の境界）

| 廃止する | 残してよい / 後続 |
|:---|:---|
| `AUTH_URL` / `WORLD_URL` / `ASSETS_URL` / `ZENOH_CONNECT` のテスター手編集必須 | 開発者・CI の env override |
| README-testers / windows-tester-zip の「example → local をメモ帳で URL 設定」手順 | Home での login（UI） |
| Start.cmd が URL 未設定で失敗する前提 | `AUTH_USER`/`AUTH_PASSWORD` を **seed 専用任意パス**に退避（本 Slice Phase 5）。サイレント login は CI 用に維持（x0018） |

`ROOM_ID` の UI 化は本 Slice に含めない（Connect = サーバ端点のみ。room 選択は Home / 後続）。

## 成功条件

1. Desktop で Connect（簡易、または詳細で Auth→Assets→World→relay）を Apply → Home login → 入室できる
2. world 配下に relay を `+` で足し、選択を切り替えて（未入室 Apply 後）別端点に接続できる
3. 再起動後も永続値が効く。env 付き CI 経路は壊さない
4. テスター手順が Connect UI 前提になり、`env.local.cmd` の URL 編集が必須でなくなる
5. `cargo test -p client`（関連 unit）/ clippy 緑。手動煙は運用者正本

## やらない（本 Slice 外）

- 連合: S2S・他運営 World 一覧・ディープリンクによる **セッション上書き**（拠点 Connect とは別 Slice）
- `alchemy-node-info.json` / 公認リレー自動発見・SLA 監視切替
- Ping 最良への自動接続、ルーター停止時の上から順フェイルオーバー（リスト優先順は本 Slice で保持）
- 複数 relay への同時接続
- join-only 起動モード本体（ローカル host を立てない配布）— 必要なら後続 Slice
- `ROOM_ID` / visibility / ワールド一覧 UI
- TLS ルート CA の UI 編集（`ZENOH_TLS_ROOT_CA` は当面 env / 同梱のまま）
- 認証・署名・門番の緩和

## 関連

- 先行: [x0018](x0018-client-auth-session.md)（Home フォーム分離）· [x0032](x0032-launcher-minimal.md) · [x003c](x003c-assets-required-mesh.md)（seed / assets 必須）
- 配布: [windows-tester-zip.md](../../0_doc/manual/windows-tester-zip.md) · `.scripts/distribution/`
- Zenoh: [zenoh-router.md](../../0_doc/architecture/zenoh-router.md) · `client/zenoh-connect`

← [目次](../README.md)
