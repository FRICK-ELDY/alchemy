# x003c — ZIP/launcher: Coin mesh は assets DL 必須

> **ID:** `x003c` · **slug:** `assets-required-mesh` · **status:** `done` · **area:** `[client, assets, host, launcher, docs]` · **depends_on:** `[x0032, x0003, x0027]`  
> **優先度:** **P0** — テスター ZIP の正規経路

目標: Windows テスター ZIP / launcher 起動でも、Coin GLB を **SceneManifest の blake3 → assets `GET /api/v1/blobs/...`** で必ず取得する。`fixtures/Coin_PolygonalMind.glb` は **assets への seed ソース**であり、release client の実行時フォールバックにしない。

根拠: ZIP 煙で Manifest 取り逃し → fallback blake3 → assets **404** → CI 絶対パス fixture で exit 1。

関連: [windows-tester-zip.md](../../0_doc/manual/windows-tester-zip.md) · [improvement-plan-2026-10-28.md](../../0_doc/evaluation/improvement-plan-2026-10-28.md) **P0-7** · P3-3（fixture debug 限定）へ接続

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| 正規 SceneManifest 受信後、記載 blake3 で assets から DL し mesh 表示 | fixture 同梱を client 実行時フォールバックとして残すこと |
| Start 成功時は常に `ensure-demo-coin.ps1` で CAS に demo coin を seed（AUTH_USER+PASSWORD または TOKEN 必須） | AUTH 無しでの Start 成功 / R2 可用性 |
| host が Presence 割当時に Manifest を再 put（late subscribe 向け） | Zenoh 保持（非保持のまま） |
| release: assets 失敗はエラー（debug のみ `ALCHEMY_ALLOW_FIXTURE_FALLBACK`） | UI 未ログイン時の fixture 表示（Home ログイン前） |

## 設計

1. **Manifest** — host: 新規割当 **または** session `jti` 回転時のみ SceneManifest を再 put（reclaim/leave では出さない）。client: auth join は Manifest blake3 必須（未着は bail；待ち 2s）。blake3 は snapshot GET 完了前に送る
2. **assets** — Start は `AUTH_*` 必須；`ensure-demo-coin.ps1` が fixture を POST；client は GET のみ
3. **fixture** — ZIP 同梱は seed 用（＋未認証 Home プレースホルダ）。auth release runtime fallback 禁止

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 + P0-7 記載 | done |
| 1 | host presence-join Manifest + client assets 必須 + ensure-demo-coin | done |
| 2 | fixture fallback を debug 限定と文書一致（P3-3 合流） | done（release 禁止 / debug は `ALCHEMY_ALLOW_FIXTURE_FALLBACK`；placeholder+timeout は → [x0048](x0048-asset-placeholder-http-timeout.md)） |

## 完了条件

- 展開済み ZIP で Start → ログに `downloaded blob via assets GET` と Manifest blake3、fixture fallback 無し
- assets `GET .../blobs/<manifest-blake3>` が 200
- 保証表と README-testers / windows-tester-zip が一致
