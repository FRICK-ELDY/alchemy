# x0080 — host handover / takeover + session dormant

> **ID:** `x0080` · **slug:** `host-handover-and-takeover` · **status:** `planned` · **area:** `[world, client, system_ui, host, docs]` · **depends_on:** `[x007c, x007e, x0019, x0022, x0032]`  
> **優先度:** 体験本線（Public Sessions の抜け殻解消 · owner と計算ホスト分離 · Graceful Handover → Manual Takeover）

目標: **永続オーナー (`owner_sub`)** と **現計算ホスト (`active_host_sub`)** を分離し、ホスト正常終了時は同室候補への **Graceful Handover**、候補なし／突然死後は **session を `dormant`** にして稼働一覧から外す。Phase 2 で Waiting からの **Manual Takeover** を足す。Room 定義行は即削除しない。

根拠: 手通しで Public Sessions に host 不在 Room が残留 · [x007c](x007c-worlds-catalog-join-host.md) 非保証「guest→host / remote host 委任」· launcher 方針「Handover stub → 失敗時は部屋終了」

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| Room に `active_host_sub`（nullable）と `state`（`active` \| `dormant`）。`owner_sub` は Handover / Takeover で変わらない | Room 行の物理 DELETE；scene / inventory 履歴の自動 GC |
| **昇格・休止の権威は world**（`POST …/handover`・`POST …/takeover`・lease 失効）。Zenoh `HandoverNotice` はヒントのみ（偽 Notice 単独では Host Token を取れない） | Room 鍵 AEAD 下での「同室者に読めない秘密チャネル」 |
| Host 用 Room Token 発行は `caller_sub == active_host_sub`、または `dormant` から owner が再開する経路、または Phase 2 で CAS 成功した takeover 直後の caller に限定 | guest Token のまま host HTTP / scene 書込；world host API の PoP（DPoP 相当）本体（別 Slice） |
| 候補なしの Graceful 終了、または host lease / liveliness TTL 超過 → `active_host_sub = null`・`state = dormant`。Public / 稼働一覧は **`state = active` のみ**（mine / Organizing は dormant 定義を見せてよい） | カタログの実 Frame 到達保証；best-effort 以外の「人数」表示 |
| Phase 1: 現 `active_host`（または owner）が `handover` で `target_sub` 指名 or `null` 休止。指名先は Server Operator（または同等の host 起動ゲート）ON の Desktop のみ自動起動を試みる；拒否・失敗なら Waiting +（Phase 2）Takeover | XR 単独での host 起動；ゲート OFF 端末への強制 spawn；ポーズ／Grab 中オブジェクトのシームレス継承 |
| Phase 2: Waiting で `takeover`（**CAS**: expected `dormant` または lease 切れ `active`）。同時立候補は一方のみ成功 | 入室維持のままの権限モデル変更以外のスマートホスト移譲；運営 headless 登録 API 本実装 |
| Leave（**ローカル host 稼働中のホスト側**）は終了前に `handover` を呼ぶ（候補なしなら dormant）。現行「Leave で host 維持」は **ホスト本人の Leave には適用しない**（ゲスト Leave / 非ホストの Leave は host を止めない） | Exit Client only の復活；完全無停止の frame 連続 |
| visibility 変更・部屋削除相当は引き続き `owner_sub` のみ | friendship 変更 alone での host 剥奪 |

---

## 設計（要点）

```
owner_sub ──永続──► Room 定義（dormant でも残る）
active_host_sub ──lease──► Session active（Public に出る）
handover(target) ──world──► active_host_sub = target | dormant
Zenoh HandoverNotice ──hint only──► 指名先が world を確認して Host Token
```

### データ（world）

| フィールド | 意味 |
|:---|:---|
| `owner_sub` | 永続オーナー（既存） |
| `active_host_sub` | 現計算ホスト。無しは `null` |
| `state` | `active` \| `dormant`（default `dormant`；初回 Host Token / Start で `active`） |
| host lease | `active` 中の心跳期限（実装は DB 列またはメモリ台帳。TTL 超過で dormant） |

### API

| 経路 | 認可 | 効果 |
|:---|:---|:---|
| `POST /api/v1/rooms/:id/handover` `{ target_sub }` | caller = `active_host_sub` または `owner_sub` | 指名 → `active`；`null` → dormant |
| `POST /api/v1/rooms/:id/takeover` | Visibility ACL + CAS（dormant / lease 切れ） | `active_host_sub = caller`・`active` |
| Host Token 発行 | 上記「保証する」の host 条件 | 不一致は 403 |
| `GET /api/v1/rooms` | 既存 visibility | 要約に `state` / `active_host_sub`。稼働投影は `active` のみ（クエリで mine は dormant 可） |

### client

1. **ホスト Leave / Quit（Phase 1）**: Presence 候補から指名（自動先頭可）→ `handover` →（任意）`HandoverNotice` → 自 `HostSupervisor` 停止。
2. **指名受信**: Notice または Waiting 後に world を見て `active_host_sub == self` → Host Token → ゲート ON なら spawn → Live。
3. **突然死（Phase 2）**: lease 失効で world が dormant／Waiting → UI「ホストを引き継ぐ」→ `takeover` → spawn。
4. **Public Sessions**: `state == active` のみ。dormant は Organizing / マイ・ルーム枠。

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 / backlog（本ファイル） | **planned** |
| 1 | world: 列 + lease + Host Token 門番 + `handover` + カタログ `state` フィルタ；単体 | planned |
| 2 | client: ホスト Leave = handover；Public は active のみ；dormant を mine に | planned |
| 3 | client: 指名検知 → ゲート付き自動 Host；docs 保証表 | planned |
| 4 | Phase 2: `takeover` CAS + Waiting CTA；crash→lease dormant E2E | planned |

---

## 完了条件

### Phase 1（〜Phase 3）

1. 一人 Host → Leave で `dormant`；Public Sessions から消える。Room 定義は owner から再 Host 可。
2. Host A + Guest B 同室 → A が B を指名して Leave → B が（ゲート ON 時）host 起動し Live 復帰；`owner_sub` は A のまま。
3. Zenoh だけ偽 `HandoverNotice` を流しても、world 上 `active_host_sub` でない caller は Host Token を取れない。
4. host 無通信が lease TTL を超えると `dormant`（crash で handover 未呼びでも幽霊 `active` が残らない）。

### Phase 2（Phase 4）

5. Host kill → Guest Waiting に Takeover CTA；成功一方のみ；Live 復帰。
6. 誰も Takeover せず全員 Leave（または TTL）→ `dormant` のまま Public に出ない。

## やらない（本 Slice）

- assets / world の別リポ化・公開体裁
- host API の PoP（別帯）
- 運営 headless 常時登録の本実装（dormant 回避の正式経路は後続）
- sim 状態（pose / hold）の完全スナップショット移譲
- Proprietary Spot-Check

## 起動（手通し）

1. 日常スタック + Desktop client（Server Operator ON）。
2. A が Create / Host → Public に `active` 行。
3. B が Join → A Leave で指名 → B が Host 起動し空間継続。
4. A 単独 Host → Leave → Public から消え、Organizing / mine から再 Host 可。
5. （Phase 2）A の host を kill → B が Takeover。

← [目次](../README.md)
