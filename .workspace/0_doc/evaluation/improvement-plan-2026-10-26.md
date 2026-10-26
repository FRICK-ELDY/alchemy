# Alchemy — 改善提案書（まとめ / 2026-10-26）

> 根拠: [specific-weaknesses-2026-10-26.md](specific-weaknesses-2026-10-26.md)  
> 目的: 採用マイナス点を優先度付きで潰し、次回評価で正味を押し上げる。

---

## 方針

1. **公開面の穴を先に塞ぐ**（Tunnel / host_control / シード）— コード量少・被害大
2. **デモで壊れる機能バグ**（Room Token 期限・JWKS 同期）
3. **門番の価値命題**（host role・assets GET・Quota）
4. **ワイヤ上の身元**（liveliness・Token・WAN TLS）
5. **CI 再現性・依存監査・E2E**
6. **定義 vs 実行**（Manifest 駆動）はスライス単位で

---

## P0 — 今すぐ（公開・可用性）

| ID | 項目 | 根拠スコア | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P0-1 | Tunnel は release のみ。`/dev/*` 非公開 | -4 | `docker-compose.prod.yml`、edge README を prod 前提に。Access または ingress 404 | 公網から mailbox/dashboard に届かない |
| P0-2 | `host_control` 無効化 or ローカル IPC | -5 | feature off 既定、または named pipe。Zenoh 経路は署名必須 | WAN peer から host を止められない |
| P0-3 | release でシード資格情報 fail-closed | -3 | `#[cfg(debug_assertions)]` / feature。未設定はエラー | release バイナリに alice/Password1 経路なし |
| P0-4 | assets test が `.env` を無視 | -3 | `runtime.exs` で `config_env() != :test`、CI で `ASSETS_CAS_BACKEND=local` 強制 | `make ci-assets` が R2 設定下でも緑 |

---

## P1 — デモ信頼性・入力ゲート

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P1-1 | Room Token 自動更新（TTL の ~80%） | -3 | client タイマー再取得。access は refresh | 入室 10 分以上操作可能 |
| P1-2 | JWKS をバックグラウンド + stale-while-revalidate | -3 | 入力スレッドから HTTP 除去。ureq タイムアウト。未知 kid クールダウン | world 一時ダウンでもキャッシュ鍵で入力継続 |
| P1-3 | move/aim magnitude・Grab 距離 clamp | -3 | 単位ベクトル化、距離ゲート、テスト | `1e30` 入力で発散しない |
| P1-4 | world: host role は owner のみ | -3 | Room.owner_sub、role 自己申告禁止 | guest JWT では host Token 不可 |
| P1-5 | assets GET 方針の文書化 **または** grant | -3 | 短期: architecture に capability モデル明記。中期: BlobGrant | 保証表とコード一致 |

---

## P2 — ワイヤ身元・同期契約

| ID | 項目 | 根拠 | 作業イメージ | 完了条件 |
|:---|:---|:---|:---|:---|
| P2-1 | liveliness 解放を検証済みセッションに紐付け | -3 | jti ハッシュ or 最終検証 input からの猶予 | 他人 sub の偽 Delete で退室できない |
| P2-2 | Room Token を毎フレーム載せない | -3 | 初回ハンドシェイク + セッション MAC / DPoP 風 | バス購読だけではなりすませない |
| P2-3 | WAN TLS（または QUIC）+ 最低限 ACL | -4 | config.wan + 手順書更新 | 平文 TCP のみの公開をやめる |
| P2-4 | client: finite / sequence / 署名検証 + snapshot | -3 | Frame ゲート、欠落時 queryable | 偽 Frame で NaN 描画しない／late join 収束 |
| P2-5 | PROTOCOL_PIN を Manifest/Presence に載せる | -2 | 不一致で入室拒否 | 世代違いを検出可能 |

---

## P3 — 品質基盤・負債

| ID | 項目 | 根拠 | 作業イメージ |
|:---|:---|:---|:---|
| P3-1 | Quota 原子的条件更新 | -2 | `UPDATE … WHERE used+? <= limit` |
| P3-2 | Refresh `FOR UPDATE` / 条件付き revoke | -2 | accounts.refresh |
| P3-3 | `/blobs` で Plug.Parsers を外す | -2 | JSON PUT が空にならないテスト |
| P3-4 | `alchemy_auth_client` で JWKS 共通化 | -2 | world/assets 複製解消 + kid クールダウン |
| P3-5 | ClientIp + TRUSTED_PROXIES / CF-Connecting-IP | -2 | edge README 必須手順 |
| P3-6 | `mix hex.audit` / `cargo deny` を CI gate | -2 | hackney 解消または差し替え |
| P3-7 | Compose 縦 E2E（login→token→input→frame） | -2 | smoke の vacuous success 廃止 |
| P3-8 | Mutex poison は fatal | -2 | 空 Presence 継続をやめる |
| P3-9 | UI HTTP をワーカーへ | -2 | desktop click 非ブロック |
| P3-10 | room_id validator を protocol 正本に統一 | -2 | 共有テストベクタ |
| P3-11 | RenderFrame に quaternion | -2 | PIN 上げ |
| P3-12 | 誤コミット `-o` 削除 | -1 | `git rm -- -o` |
| P3-13 | protoc 全 proto・Rust cache | -1 | workflow 修正 |
| P3-14 | undocumented_unsafe_blocks deny | -2 | XR 経路から段階的 |

---

## P4 — アーキテクチャ整合（スライス推奨）

| ID | 項目 | 根拠 | 提案との接続 |
|:---|:---|:---|:---|
| P4-1 | SceneManifest 駆動で host/client 定数解消 | -3 | 署名 Manifest 提案 |
| P4-2 | shared-sim / EntityKind | 保守性 | 提案 |
| P4-3 | trust-boundary threat model を architecture へ | 文書 | GPT 提案 |
| P4-4 | launcher 最小実装 | 対象外→実装 | 提案 |
| P4-5 | Friendship → 公開範囲 | 目標 | 提案 |

---

## 推奨スライス順序（例）

```
x001e  P0（prod Tunnel / host_control / seed / assets CI）
x001f  P1（Token refresh / JWKS bg / magnitude / host role）
x001g  P2 前半（liveliness / WAN TLS）
x001h  P2 後半（session binding / client verify / snapshot）
x001i  P3 まとめて（Quota/Parsers/audit/E2E）
x001j  P4 Manifest 駆動（定義 vs 実行）
```

---

## 次回評価で見る完了定義

- [ ] `make ci`（assets 含む）が開発者 `.env` ありでも緑
- [ ] 公網手順が release のみ、mailbox 非公開
- [ ] WAN から host_control / 偽 liveliness でセッション破壊できない（テストまたは手順で証明）
- [ ] 入室 10 分後も入力可能
- [ ] architecture の保証表に assets GET モデルと Zenoh 信頼境界が追記されている
- [ ] 採用マイナスの P0–P1 が「解決済み」とコード再検証できる

---

## やらないこと（この計画の範囲外）

- billing / market / 完全 Spatial VCS / Dual Editor / Quest LOD（提案のままロードマップ）
- 旧 Elixir 権威実行への回帰
