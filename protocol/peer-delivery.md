# Peer delivery（アプリ向け配送契約）

> **Slice:** x0024 · **正本クレート:** [`protocol/`](.)  
> Zenoh は配送部品。本ファイルは **アプリが何を保証し何を保証しないか** の対応表である。

## 語彙

| 語 | 意味 |
|:---|:---|
| **latest** | 古い値は捨ててよい。届いた最新だけが意味を持つ。中間欠落の再送はしない |
| **request/response** | 要求に対する一回の応答（フル状態など）。連続レートは保証しない |
| **best-effort** | 到達も順序も保証しない。受理側の追加ルール（例: `intent_seq`）は別契約 |
| **Blob（参照）** | 大容量はハッシュ参照。バイト本体は Zenoh 外（HTTP CAS 等） |
| **受信成功** | peer / SDK が長さとデコード可能なバイトを受け付けたことまで。アプリ適用・保存完了は含まない |
| **`not-sent`** | 自側が未送出と分かっている失敗 |
| **`unknown`** | 送出後に ACK 欠落・切断などで結果不明。**別 ID として自動再送しない** |
| **欠落デルタ** | `frame` の tick が飛んだ状態。再送しない。ローカル entity 表は **unknown** → `snapshot` 取り直し |

確実配送（sendTo 相当・event ID・ACK）は **未設置**。編集・インベントリ・課金が載るときに本表へ行を足す。語彙だけ先に固定する。

## 現行キー対応表

キー式: `alchemy/dev/room/{room_id}/…`（x0017）。

| キー / 経路 | クラス | 保証する | 保証しない |
|:---|:---|:---|:---|
| `…/frame` | **latest**（デルタ） | 届いたデルタを **byte/count 上限**（x003b）→ `host_sig` 検証・finite（TRS 含む・x0029）・tick 規則の後に適用できる。**publish された tick は連番**（空は未送かつ未加算）。回転/scale 変化も upsert | 物理 Hz ごとの空 tick・欠落の再送 |
| `…/snapshot` | **request/response**（フル pose） | late join / 再同期で entity 表を置換収束（`upserts` 非空・TRS 同スキーマ；**decode 上限** x003b） | 連続 query のレート・空応答時の即時成功 |
| `…/manifest` | **latest**（低頻度＋再送） | 届いた最新を **byte 上限 → decode → count/string 上限 → `protocol_pin` → `room_id` → `world_seal`（blake3）→ `host_attestation` → `(host_generation, revision)` 単調ゲート** の順で検証後に採用（x0026 / x003a / **x003b**）。**同一 generation 内は `host_pub` 固定**（不一致・鍵欠落は discard）。entries が配置・Grab・mesh・**初期 TRS** の定義正本（x0027 / x002c）。host は seal / attestation を TTL 80% で更新し、失敗時は公開停止 | shared-sim / EntityKind・欠落再送・適用完了 ACK・stale seal の継続公開・二重 host の generation フリップフロップ UX；router max-payload |
| world `GET …/scene` | **request/response**（HTTP） | host が Room 作成後に DemoScene（初期 quat/scale 含む）を取得し Rapier / Manifest の入力にする（x0027 / x002c） | guest 取得・シーン編集・collider の非一様リサイズ |
| `…/presence` | **latest**（低頻度） | 届いた割当表を **byte/count 上限**（x003b）+ pin + **`host_sig`**（attested `host_pub`）+ **Manifest `host_generation` 一致 / revision 単調** 後に採用（x0025 / x003a）。Presence 単独で generation を上げない | world 署名・途中欠落の補完 |
| `…/input` | **best-effort Intent** | host が **byte/token 上限**（x003b）後に受理した Intent について `intent_seq` 単調。bind 時 `role`→caps で move/grab/leave をマスク（x002a） | put 成功＝適用成功、ACK、確実配送、caps のワイヤ明示 |
| `…/host_control` | **best-effort command**（既定 OFF） | `ALCHEMY_HOST_CONTROL=1` のとき到達は best-effort | 認証・ACK |
| `…/alive/{sub}/{jti}` | liveliness | Delete で host が jti 一致時 reclaim（x0022） | WAN ACL・他人 Delete の完全防御 |
| assets HTTP CAS | **Blob（参照）** | blake3 参照の取得は assets 層の契約 | Zenoh 上の Blob 配送 |

## `frame` 欠落と再同期（D3）

`frame` は latest デルタであり、**欠けた tick の再送はしない**。

### host（ワイヤ tick の付け方）

- 空デルタ（upsert/remove なし）は **publish しない**
- そのとき **`net_tick` も進めない**（x0024）
- したがって **publish された frame 同士の tick は連番**（`FRAME_TICK_GAP_RESYNC = 1` と整合）
- snapshot キャッシュの pose は空スキップ中も更新してよいが、**tick は最後に publish した値**（次の未 publish `net_tick` を載せない）。未 publish だと DropStale で直後のデルタが死ぬ
- まだ一度も publish していない間は、空ホールドで ready snapshot を作らない（`upserts` 空のまま → `reply_err`）

### client

| client 状態 | 動作 |
|:---|:---|
| `tick <= last` | 捨てる（snapshot 適用後も同じ） |
| `last < tick <= last + FRAME_TICK_GAP_RESYNC` | デルタ適用（既定 `FRAME_TICK_GAP_RESYNC = 1` → 連番のみ） |
| `tick > last + FRAME_TICK_GAP_RESYNC` | デルタ未適用。ローカルは unknown → `…/snapshot` get で置換 |

**禁止するのは「未 publish の次 `net_tick`」を snapshot に載せること**（空ホールド中）。publish 直前にキャッシュへ同じ tick を書き、直後に同 tick のデルタを put するのは許容（フル pose のため、client が同 tick を DropStale しても欠落にはならない）。あってはならないのは、空スキップ中に **これから送る予定の tick** を snapshot だけが先に広告し、後続デルタが同番号で死ぬケース。

gap → snapshot の client クールダウン **500ms は `fetch_snapshot` の成否共通**（失敗しても次の gap 再試行まで同じ待ち。負荷抑制のための意図）。

host の snapshot queryable 契約は x0023 のまま（空 `upserts` は未準備 `reply_err`）。

定数: [`FRAME_TICK_GAP_RESYNC`](src/lib.rs)（`alchemy-protocol`）。

## 上限・診断（測っていない値は null）

| 項目 | 値 |
|:---|:---|
| `FRAME_TICK_GAP_RESYNC` | `1`（**publish された** frame の連番必須） |
| 論理メッセージ上限（バイト） | null |
| snapshot リトライ間隔 | Manifest 再送周期に依存（host 約 5s）。gap 検知時は get。client クールダウン **500ms・成否共通**（失敗時も 500ms 待つ） |
| 経路別送受信数 / 再送 / 待ち行列 / RTT | null（未計測） |

## 将来行（まだ作らない）

| 想定経路 | クラス | メモ |
|:---|:---|:---|
| 編集 / インベントリ / 課金 | 確実配送 | event ID・世代（jti）固定・`not-sent`/`unknown`。ACK は「受け付け」まで |
| 参加者向け一回通知 | event | 途中参加には再生しない |

## 関連

- Wire 型・キーヘルパ: [`src/lib.rs`](src/lib.rs)
- Architecture 要約: [`.workspace/0_doc/architecture/protocol.md`](../.workspace/0_doc/architecture/protocol.md)
- Slice: [x0024-delivery-class.md](../.workspace/roadmap/slices/x0024-delivery-class.md)
