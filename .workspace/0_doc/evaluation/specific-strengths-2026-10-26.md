# Alchemy — プラス点統合一覧（まとめ / 2026-10-26）

> 第1評価者（Claude Opus 5.5）と第2評価者（GPT-5.6 Sol）の独立評価を統合。
> 両者が高く評価した項目を先に置き、片方のみの卓越点は注記する。

## 採点基準

| 点数 | 基準 |
|:---:|:---|
| +1 | 正しく実装されている。問題はないが特筆するほどではない |
| +2 | 業界の一般的なベストプラクティスに沿った、良い設計判断 |
| +3 | 同規模・同種プロジェクトの平均を明確に上回る実装 |
| +4 | プロダクションレベルのゲームエンジン・OSSと比較しても遜色ない実装 |
| +5 | このクラスの個人プロジェクトでは見たことがないレベルの卓越した実装 |

---

## 両者が高く評価した強み

### auth-server

- **Refresh Token ファミリー回転・再利用検知 / アカウントライフサイクル** `+4`
  > ハッシュ保存、family 一括失効、grace、行ロック付き reset/change/deactivate。Keycloak / OAuth BCP 相当。Opus は rotation を、GPT は lifecycle 全体を主軸に採点したが同一クラス。
  > 対象ファイル: `auth-server/lib/auth/accounts.ex`
  > 評価者: Opus `+4` / GPT `+4`

- **RS256 + kid（thumbprint）+ 複数鍵 JWKS ローテーション** `+3`
  > 対象ファイル: `auth-server/lib/auth/token/keys.ex`
  > 評価者: 双方 `+3`

- **多軸レート制限 + 列挙耐性 + 失敗理由の内外分離** `+3`
  > 対象ファイル: `auth-server/lib/auth_web/plugs/rate_limit.ex`, `authenticate.ex`
  > 評価者: 双方 `+3` 帯

### protocol / host / client

- **独立 protocol クレート + Manifest/Frame/Input/Presence 責務分離** `+3`
  > GLB 非搭載・f64 姿勢・キー式ヘルパとテスト。Resonite 系と同筋。
  > 対象ファイル: `protocol/`
  > 評価者: Opus チャネル `+3` + crate `+2` / GPT SSoT `+3` + 分離 `+3`

- **Presence 解放の三重化（Leave / liveliness / Intent TTL）** `+3`
  > 対象ファイル: `client/host-server/src/main.rs`
  > 評価者: 双方 `+3`

- **60Hz 物理 / 30Hz 配信分離 + SoA + Room Token 後の finite gate** `+3`
  > 対象ファイル: `client/host-server/src/sim.rs`, `main.rs`
  > 評価者: 双方 `+3` 帯

- **Desktop / OpenXR surface fallback + Facet が OpenXR/Room を知らない** `+3`
  > 対象ファイル: `client/app/src/surface/mod.rs`, `system_ui/facet.rs`
  > 評価者: Opus Facet `+3` / GPT surface `+3` + Facet `+2`

- **assets 取得後 BLAKE3 + Pod GPU layout** `+3`（GPT）/ 同等評価（Opus）
  > 対象ファイル: `client/app/src/main.rs`

### assets / world / edge

- **CAS メタと不変バイナリ分離 + local/S3 切替 + 302 署名 URL** `+2`〜`+2`
  > 対象ファイル: `assets-server/lib/assets/cas.ex`, `cas/s3.ex`
  > 評価者: 双方合意

- **短命 Room Token の claim 設計 + Bearer/公開パイプライン分離** `+2`
  > 対象ファイル: `world-server/lib/world/token.ex`, `world_web/router.ex`
  > 評価者: 双方合意

- **LAN/WAN config 分離・Tunnel テンプレ・手順書** `+2`
  > 対象ファイル: `zenoh-router/`, `edge/cloudflared/`
  > 評価者: 双方合意

---

## 第1評価者が特に高く評価した点（第2は弱め or 未採点）

- **architecture「保証する／しない」とコード・スライスの一致** `+4`
  > 文書規律として個人 OSS では稀。まとめでも採用。
  > 対象ファイル: `.workspace/0_doc/architecture/*.md`

- **実機コントローラ render model（OpenXR）** `+3`
  > 対象ファイル: `client/app/src/surface/xr/render_model.rs`

- **auth のメール確認・reset のトランザクション設計の細部** `+2`
- **匿名トークン既定 OFF・room_id 厳格検証** `+1`〜`+1`
- **local CAS の tmp→rename** `+1`
- **スライス運用・roadmap の自己改善サイクル** `+3`

---

## 第2評価者が特に高く評価した点（第1と重複しつつ独立採点）

- **host 物理・Grab・slot の振る舞いテスト 19 件** `+2`
  > 当日 WSL で 19/19 通過。まとめでも DX/テストの加点根拠として採用。
- **サービス別 CI と実用単体群（auth 107 / client 55 / protocol 7）** `+3`
- **定義・門番・実行・endpoint の大枠がコードと一致** `+2`
- **WSL/Docker/README の単一導線** `+2`

---

## 対象外（減点も加点もしない）

- `launcher/` — `.gitkeep` のみ
- billing / market-server — ディレクトリなし
- verifier / Formula VM / OSC / Dual Editor / LOD / P2P / Wake-on-Join — 目標のみ（提案へ）

---

## 採用プラスの読み方

Opus のプラス合計（+96）は文書規律・auth 細部・XR まで広く加点。GPT（+61）は「動く境界と検証できた範囲」に絞り気味。まとめは**合意の +3/+4 コアを正**とし、文書規律（+4）は Opus を採用して総合に反映する。
