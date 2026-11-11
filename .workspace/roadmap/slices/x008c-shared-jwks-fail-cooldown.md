# x008c — 共有 JWKS 失敗クールダウン（async miss refresh）

> **ID:** `x008c` · **slug:** `shared-jwks-fail-cooldown` · **status:** `done` · **area:** `[shared, world, assets, docs]` · **depends_on:** `[x002f, x0044]`  
> **優先度:** 評価 P2-5（auth 障害が増幅しない共有 JWKS）

目標: `alchemy_auth_client` の JWKS キャッシュを **host / client / relay と同型**にする。取得失敗でも短いクールダウンを進め、unknown-kid / empty の再取得は **GenServer 内で HTTP 同期しない**（in-flight は合流）。

根拠: [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md) **P2-5** · [opus-specific-weaknesses-2026-11-05.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-05.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 取得 **失敗** でも `last_fetch_ms` を進め、既定 `fetch_cooldown_ms`（1s）内は再 HTTP しない | host 級の soft 60s / hard 300s TTL スケジュール |
| `signer_for_kid` の miss 経路は **キャッシュだけを見て即返信**；再取得は非同期 kick（in-flight 合流） | 非同期完了前の kid が直ちに通ること（次リクエスト以降）。`AuthenticateUser` 等の **同一 HTTP 内リトライ**は呼び出し側任意（プラグは 401 一発でよい） |
| 成功後の unknown-kid 嵐も従来どおりクールダウン | 偽 kid 連打の CPU（JWT peek 自体） |
| 明示 `refresh/1` は同期取得のまま（テスト / 起動追随）。失敗時もクールダウン stamp | `Assets.World.Jwks`（world PlaceRead 用・別モジュール）の同型化 |
| Bypass 単体で「失敗後クールダウン」「miss が HTTP を待たない」を固定 | 分散ノード横断の共有カウンタ |

---

## 設計（要点）

```
signer_for_kid(kid)
  ├─ cache hit → {:ok, signer}
  └─ miss → maybe_kick_async_fetch() → lookup(cache)  # HTTP しない
         └─ in-flight / cooldown → kick しない

fetch 完了（成功|失敗）→ last_fetch_ms = now
失敗 → 旧鍵を保持（空のままなら空のまま）
```

- kick は `spawn_monitor`（TaskSupervisor 追加なし）。結果は `handle_info` で適用
- 起動 `handle_continue :initial_fetch` は同期のまま（空キャッシュの初回だけ）。失敗時も stamp
- `now_ms` 注入で決定的テスト（既存）

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + roadmap / backlog / improvement | **done** |
| 1 | `AlchemyAuthClient.Jwks` 失敗クールダウン + async miss | **done** |
| 2 | 単体テスト + README / architecture 注記 | **done** |
| 3 | improvement P2-5 done 表記 | **done** |

---

## 完了条件

1. auth が 5xx / 不通のとき、unknown kid 連打でもクールダウン内に Bypass hit が増えない
2. miss 経路の `signer_for_kid` が HTTP 完了を待たずに戻る（GenServer 直列待ちにならない）
3. 成功後 unknown-kid クールダウンの既存保証が残る
4. `mix test` in `shared/alchemy_auth_client` 緑

## やらない（本 Slice）

- soft/hard TTL の host 完全移植
- `Assets.World.Jwks` の同型化（別 Slice）
- Port プール（P2-1） / Origin Cert hash（P2-4）

## 関連

- [x002f-p3-auth-client-unsafe.md](x002f-p3-auth-client-unsafe.md)
- [x0044-client-world-jwks-cooldown.md](x0044-client-world-jwks-cooldown.md)
- [improvement-plan-2026-11-05.md](../../0_doc/evaluation/improvement-plan-2026-11-05.md)

← [目次](../README.md)
