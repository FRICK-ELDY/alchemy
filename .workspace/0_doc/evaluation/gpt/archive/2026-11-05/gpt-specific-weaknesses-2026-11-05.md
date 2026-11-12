# Alchemy 第2評価者 — 具体的な弱み（2026-11-05）

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| -1 | 改善余地あり。動作はするが設計・品質上の軽微な問題 |
| -2 | 重要な機能・設計の欠如。放置すると将来の拡張を阻害する |
| -3 | 設計上の明確な欠陥。バグ・クラッシュ・性能劣化を引き起こしうる |
| -4 | プロジェクトの価値命題を損なう重大な欠如 |
| -5 | プロジェクトの根幹を揺るがす致命的欠陥 |

## 転送・公開エッジ

### Room AEAD / WAN

- **Room 鍵が部屋寿命で固定され、退出・可視性変更後の読取権を失効できない** `-3`
  > `room_key` は Room 行に永続化され（`rooms.ex:629-640`）、Token 発行ごとに同じ鍵を全参加者へ返す（`room_controller.ex:196-223`）。一方 WAN ACL は role CN だけを主体にして全 Room の `*/*/manifest|frame|presence` 購読を許す（`config.wan.json5:142-172`）。したがって、一度入室した主体は鍵を保存し、後日ほかの公開 Room で得た有効 guest 証明書でも wildcard 経由で旧 Room の暗号文を受け、可視性変更後も復号できる。AEAD は現在の他室参加者には効くが、過去参加者の revoke 境界にはなっていない。
  > 改善方針: `room_key_epoch` を導入し、退出・visibility 縮小・権限剥奪時に鍵を回転する。ワイヤ AAD と Token に epoch を束縛し、旧 epoch の publish/subscribe を拒否する。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `world-server/lib/world_web/controllers/room_controller.ex`, `relay-server/config.wan.json5`

- **Relay Agent 互換経路は依然として秘密鍵を cleartext HTTP 応答へ載せる** `-2`
  > 推奨の world CSR 経路は private key を返さないが、互換 Agent は `AllocateResponse.key_pem` を持ち（`lib.rs:79-88`）、サーバー側で RSA 鍵を生成して JSON 応答へ格納する（`lib.rs:483-566, 697-706`）。非 loopback bind は警告だけで起動を継続する（`lib.rs:230-239`）。README 自身も「CSR のみ発行 / key_pem 非返却」を非保証に置く。フォールバックを使う運用では前回の秘密鍵配送問題は open のままである。
  > 改善方針: Agent も CSR-only にし、非 loopback cleartext bind は起動拒否する。互換が不要なら allocate API 自体を廃止し world 発行へ一本化する。
  > 対象ファイル: `relay-agent/src/lib.rs`, `relay-agent/README.md`

- **Allocation 発行に時間窓レートがなく、持続的な RSA/OpenSSL CPU 消費を止めない** `-1`
  > 接続数 32・同時 allocate 8・直列 `cert_issue_lock` は同時資源を抑えるが（`lib.rs:35-43, 145-151, 292-304, 483-489`）、sub / relay / IP 単位の token bucket はない。攻撃者が正規に多数の異なる `jti` を取得できる場合、300 秒窓で発行を継続できる。前回の「無制限 thread」は閉じたが、発行 throughput の policy は未実装である。
  > 改善方針: 検証済み `sub` と接続元 IP に短窓・長窓 bucket を設け、拒否と発行時間を既存 metrics に出す。
  > 対象ファイル: `relay-agent/src/lib.rs`

## 運営側コア

### world-server

- **Scene append / delete が revision CAS なしの read-modify-write で更新を失う** `-3`
  > `append_scene_entry/2` と `delete_scene_entry/2` は Room を読み、次の scene を作り、`Room.set_scene` するだけで、読み取った revision を WHERE 条件に含めない（`rooms.ex:180-216`）。host 側では spawn と despawn が別スレッドから HTTP を実行し（`inventory_spawn.rs:160-183`, `inventory_despawn.rs:104-130`）、応答後のローカル適用だけを revision 単調ゲートで守る（`room_scene.rs:129-145`）。同時要求が同じ revision を読めば DB では後勝ちとなり、一方の spawn/despawn が恒久的に消える。
  > 改善方針: `UPDATE ... WHERE revision = expected_revision` の楽観 CAS と競合再試行、または Room 単位の直列化を world 側で実装する。競合 append+delete の統合テストを追加する。
  > 対象ファイル: `world-server/lib/world/rooms.ex`, `client/host-server/src/inventory_spawn.rs`, `client/host-server/src/inventory_despawn.rs`

- **host credential / scene 系 HTTP が rate limit パイプライン外にある** `-2`
  > IP+user rate limit は auth JWT の create/token/visibility scope だけに適用される（`router.ex:12-21`）。Host Room Token を使う `host_keys`、`manifest_seal`、scene append/delete、PlaceRead は `[:api]` のみである（`router.ex:24-38`）。有効 Token の漏えい・悪用時に署名、常駐 hash Port、DB 更新を直接連打できる。
  > 改善方針: Room Token 検証後の `room_id+jti` / IP bucket を別 pipeline に置き、操作別上限を設定する。scene mutation は構造上限も併用する。
  > 対象ファイル: `world-server/lib/world_web/router.ex`, `world-server/lib/world_web/plugs/rate_limit.ex`

- **Room 所有数上限は並行 create を厳密に止めない** `-1`
  > `SELECT COUNT(*)` 後に別途 insert し、モジュール文書も「concurrent races may overshoot」と明記する（`rooms.ex:7-14, 579-632`）。時間レートは増幅を抑えるが、構造上限の保証にはならない。
  > 改善方針: owner ごとの quota row を条件付き更新するか、DB trigger / serializable transaction で count と insert を一体化する。
  > 対象ファイル: `world-server/lib/world/rooms.ex`

### client/app

- **外部 GLB の展開後資源上限がなく、100 MB 入力から大規模確保できる** `-2`
  > assets は upload bytes を 100 MB に制限するだけで、client は `gltf::import_slice` 後に positions / indices / images を上限なしで `Vec` へ収集する（`mesh.rs:117-151`）。脅威モデルが要求する頂点数・画像ピクセル・展開後 bytes 上限は実コードにない。BLAKE3 は改ざんを防ぐが、正当に登録された圧縮爆弾や巨大 GLB の OOM は防がない。
  > 改善方針: decode 前に accessor count・bufferView 範囲・画像寸法を検査し、頂点・index・texture pixels・総展開 bytes にハード上限を設ける。
  > 対象ファイル: `client/app/src/mesh.rs`, `assets-server/config/config.exs`

## 横断品質

### テスト / E2E

- **WAN 否定系は実 host-server の publish 経路まで到達しない** `-1`
  > trust-probe は GUI と共有する trust gate を通る点で前回より大幅に改善したが、WAN overlay は「host-server Frame path」を明示的に非保証とし、probe 内で host CN の別 Zenoh sessionを作って synthetic Manifest を put する（`docker-compose.e2e.wan.yml:1-8`, `trust-probe.rs:559-611`）。credential worker、host の cert install、署名 Frame、実 client bus をまとめた回帰はまだない。
  > 改善方針: WAN compose に host-server を起動し、world CSR 証明書で実 Frame を publish、headless trust consumer が受理する一往復を追加する。
  > 対象ファイル: `docker-compose.e2e.wan.yml`, `client/trust/src/bin/trust-probe.rs`

- **Windows 全 workspace テストに一過性失敗が残る** `-1`
  > 2026-11-05 の `cargo test --workspace` は `surface::xr::loader::tests::parse_library_folders_extracts_path` が `expect("parse")` で失敗し、118 passed / 1 failed だった。直後の当該テスト単独実行と全体再実行は通過したため、再現性の低い temp directory / filesystem 依存と判定する。CI の信頼性を落とすが恒常不具合とは断定しない。
  > 改善方針: 固定 temp 名を避けて一意な tempfile を使い、parser と `is_dir()` を分離して純粋テスト化する。
  > 対象ファイル: `client/app/src/surface/xr/loader.rs`

**弱み合計: -16**
