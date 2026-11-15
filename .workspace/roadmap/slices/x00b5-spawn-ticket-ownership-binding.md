# x00b5 — 配置時の所有権バインド（SpawnTicket）

> **ID:** `x00b5` · **slug:** `spawn-ticket-ownership-binding` · **status:** `planned` · **area:** `[assets, world, host, client, protocol, docs]` · **depends_on:** `[x006b, x007f, x009d, x009e, x00b2]`  
> **優先度:** [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §14.4 対策 1（現行の穴。[x00b4](x00b4-prefab-child-grant-required.md) の後に着手）  
> **関連:** [assets-threat-model.md](../../0_doc/architecture/assets-threat-model.md) ① 目標の防御 (4)

目標: scene / overlay への blake3 追記を、assets-server が署名した短命の SpawnTicket（「部屋の参加者がその blob の Grant を持つ」ことの証明）がある場合にだけ world-server が受理する。改造したローカル host-server が「hash を知っている」だけで、PlaceRead 経由の読み取り権を得る経路を閉じる。

根拠: [assets-vcs-draft.md](../../0_doc/architecture/assets-vcs-draft.md) §14.3–§14.4 · PlaceRead の `extra_blake3` 無効化は「take-out ownership binding」までの暫定措置（`Assets.PlaceRead` moduledoc）

---

## 現状の穴

| 経路 | 現状の検証 |
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

## 設計の骨子（Phase 0 で確定する）

| 項目 | 案 |
|:---|:---|
| 発行者 | assets-server。take_out 応答に同梱（`room_id` 指定時）。ワールド制作者が自分の blob を canonical に置くための単体発行 API も用意する（Grant を確認して発行） |
| クレーム | `typ=spawn_ticket` / `aud=alchemy-world-spawn` / `iss` / `sub`（take_out したユーザー）/ `room_id` / `blake3s`（上限あり）/ `jti` / `iat` / `exp` |
| 署名と公開鍵 | 既存依存（Joken / JOSE）の範囲でアルゴリズムを選ぶ。assets に JWKS を公開し、world は既存の JWKS 取得（auth と同型）で検証する。**新規依存は入れない**（必要になったら人に回す） |
| 受け渡し | client → host: `EditSpawn` にチケットのフィールドを追加（`PROTOCOL_PIN` 更新）。host → world: overlay / scene の POST 本文に同梱 |
| world の検証 | 署名・`typ` / `aud` / `iss` / `exp`、`room_id` がパスと一致、blake3 がチケットに含まれる、`sub` が部屋の参加者、`(jti, blake3)` が未使用 |
| 複数子の Prefab | 1 枚のチケットで複数の blake3 を許し、消費は `(jti, blake3)` 単位にする（子ごとに POST が分かれるため） |
| エラー | 403 `spawn_ticket_required` / `spawn_ticket_invalid`（詳細はログにも出しすぎない） |

---

## 未決（Phase 0）

- `sub` が部屋の参加者であることの判定方法（Room Token の発行記録・presence・host Token 経由の申告のどれを根拠にするか）
- 署名アルゴリズムと鍵の置き場所（Origin Cert 鍵とは分離する前提）
- 既存 scene（DemoScene や既存 canonical）の扱い。新規追記のみを対象とするか、移行期間を設けるか
- private リポジトリの Release をワールドとして公開する将来形（タグの blob 集合をまとめて証明するチケット）との整合
- チケット上限（`blake3s` の最大数・`exp`）と、x0071 / x0084 のレート制限との関係

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | 設計確定（上の未決）+ Slice / roadmap | Slice のみ **done** |
| 1 | assets: 鍵・JWKS・チケット発行（take_out 同梱 + 単体発行 API） | planned |
| 2 | world: overlay / scene 追記時の検証、`(jti, blake3)` 消費、エラー応答 | planned |
| 3 | protocol / host / client: `EditSpawn` へのチケット追加（`PROTOCOL_PIN`）、host の POST 同梱 | planned |
| 4 | 回帰・e2e・文書（world-server.md / assets-server.md / 脅威モデル ① を「現状の防御」へ昇格） | planned |

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
