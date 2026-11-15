# x00b5 — 配置時の所有権バインド（SpawnTicket）

> **ID:** `x00b5` · **slug:** `spawn-ticket-ownership-binding` · **status:** `done` · **area:** `[assets, world, host, client, protocol, docs]` · **depends_on:** `[x006b, x007f, x009d, x009e, x00b2]`  
> **優先度:** [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §14.4 対策 1（現行の穴。[x00b4](x00b4-prefab-child-grant-required.md) の後に着手）  
> **関連:** [assets-threat-model.md](../../0_doc/architecture/assets-threat-model.md) ① 目標の防御 (4)

目標: scene / overlay への blake3 追記を、assets-server が署名した短命の SpawnTicket（「部屋の参加者がその blob の Grant を持つ」ことの証明）がある場合にだけ world-server が受理する。改造したローカル host-server が「hash を知っている」だけで、PlaceRead 経由の読み取り権を得る経路を閉じる。

根拠: [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §14.3–§14.4 · PlaceRead の `extra_blake3` 無効化は「take-out ownership binding」までの暫定措置（`Assets.PlaceRead` moduledoc）— 本スライス後も `extra_blake3` は無効のまま（配置はチケット経路）。

---

## 現状の穴（実施前）

| 経路 | 実施前の検証 |
|:---|:---|
| `POST …/overlay/entries`（`RoomController.create_overlay_entry`） | host Room Token + PoP + active host。blake3 は `World.Rooms.append_overlay_entry/4` で形式と重複のみ |
| `POST …/scene/entries`（`RoomController.create_scene_entry`） | 上記 + scene writer（`owner_sub` ∪ `editor_subs`・x009d）。blake3 は形式と重複のみ |
| seal → `PlaceHashes` → PlaceRead | 部屋の参加者（role を問わない）が scene の blake3 を GET できる |
| client → host（`…/edit` の `EditSpawn.blake3`） | host は受け取った blake3 をそのまま overlay に POST する（`client/host-server/src/inventory_spawn.rs` → `room_token/room_scene.rs`） |

結果: host（ローカルで動く・改造されうる）は、別の部屋で見かけた他人の blob の hash を自分の部屋に置き、seal 後に PlaceRead で読み出せる。

---

## 保証する / しない（目標）

| 保証する | 保証しない |
|:---|:---|
| 新しく scene / overlay に追記される blake3 は、部屋の参加者がその blob の Grant を持つことを assets-server が署名で証明したものだけ | 部屋に置かれたデータの抽出防止（受け入れるトレードオフ・assets-vcs-draft §14.2） |
| チケットは `room_id` に縛られ、短命（数十秒）で、`(jti, blake3)` ごとに 1 回限り | 既存の scene 行（DemoScene など）の遡及検査 |
| チケットが無い・不正・期限切れ・別の部屋・使用済みの追記は 403（fail-closed） | 退出したユーザーのアイテムの即時撤去 |
| 署名鍵は Origin Cert の鍵と分離し、ログ・レスポンスに出さない | 鍵ローテーションの自動化・失効リスト |
| 判定は world-server / assets-server だけで行い、host-server の判定に頼らない | host-server の改造検知 |

---

## 設計確定（Phase 0）

| 項目 | 決定 |
|:---|:---|
| 発行者 | assets-server。`POST /api/v1/drive/take_out` に `room_id` 指定時に同梱。単体 `POST /api/v1/spawn_tickets`（Grant 確認後発行） |
| クレーム | `typ=spawn_ticket` / `aud=alchemy-world-spawn` / `iss=alchemy-assets` / `sub` / `room_id` / `blake3s`（最大 32）/ `jti` / `iat` / `exp`（TTL 60s） |
| 署名と公開鍵 | **RS256**（既存 Joken/JOSE）。`Assets.SpawnTicket.Keys`（Origin Cert Ed25519 と分離）。`GET assets/.well-known/jwks.json`。world は `World.Assets.Jwks` で取得 |
| 受け渡し | client → host: `EditSpawn.spawn_ticket`（`PROTOCOL_PIN` = `alchemy-protocol/0.24.0-x00b5`）。host → world: overlay / scene POST 本文に `spawn_ticket` |
| world の検証 | 署名・typ/aud/iss/exp、`room_id` 一致、blake3 ∈ チケット、`sub` が参加者、`(jti, blake3)` 未使用。consume ETS はチケット `exp` を保持し期限後に間隔 prune（TTL 延長でも再利用窓を開けない）。**CAS conflict 時のみ** claim 解放（他の追記失敗は焼いて再 take_out 必須 = fail-closed） |
| 参加者の `editor_subs` | JSON 列を `Rooms.editor_subs/1`（`ensure_scene_writer` と同デコード）で読む。`List.wrap(raw)` は使わない |
| 参加者根拠 | Room Token 発行台帳（`World.RoomParticipants`・`Token.generate` で note）**OR** room の `owner_sub` ∪ `editor_subs` ∪ `active_host_sub`。Presence / host 申告は使わない |
| 既存 scene | **新規追記のみ** fail-closed。DemoScene / 既存 canonical は遡及しない |
| 複数子 Prefab | 1 枚のチケットで複数 blake3。消費は `(jti, blake3)` 単位 |
| エラー | 403 `spawn_ticket_required` / `spawn_ticket_invalid`（詳細はログ抑制） |
| private Release 将来形 | 本スライスでは take_out + 単体発行まで。タグ一括チケットは未実装 |
| レート | world は既存 `host_scene`。assets 発行専用レートは未追加（x0071 系とは別軸） |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 設計確定（上の未決）+ Slice / roadmap | **done** |
| 1 | assets: 鍵・JWKS・チケット発行（take_out 同梱 + 単体発行 API） | **done** |
| 2 | world: overlay / scene 追記時の検証、`(jti, blake3)` 消費、エラー応答 | **done** |
| 3 | protocol / host / client: `EditSpawn` へのチケット追加（`PROTOCOL_PIN`）、host の POST 同梱 | **done** |
| 4 | 回帰・e2e・文書（world-server.md / assets-server.md / 脅威モデル ① を「現状の防御」へ昇格） | **done** |

---

## 完了条件

1. チケットの無い overlay / scene 追記が 403 になる（host Token・PoP・writer を満たしていても）
2. 別の部屋のチケット・期限切れ・使用済み `(jti, blake3)`・チケット外の blake3 がすべて拒否される
3. 正規の take_out → 配置 → seal → PlaceRead → GET の往復が通る（Desktop smoke を含む）
4. 署名鍵がログ・レスポンス・コミットに出ない
5. 新規依存を追加していない（追加が必要なら人の承認を得てから）

---

## 起動・検証メモ

```powershell
cargo test -p alchemy-protocol
cd assets-server; mix format; mix test; cd ..
cd world-server; mix format; mix test; cd ..
# from client/
cargo test -p host-server --bin host-server
cargo test -p client --bin client
```

推奨ブランチ: `feat/x00b5-spawn-ticket-ownership-binding`

← [目次](../README.md)
