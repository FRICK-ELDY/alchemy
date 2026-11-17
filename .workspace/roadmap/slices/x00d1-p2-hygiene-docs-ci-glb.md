# x00d1 — P2 衛生一括（docs/release · assets CI `_build` · GLB 寸法先読み）

> **ID:** `x00d1` · **slug:** `p2-hygiene-docs-ci-glb` · **status:** `done` · **area:** `[docs, ci, client]` · **depends_on:** `[x0085, x008a, x0093, x00d0]`  
> **優先度:** improvement-plan **P2-3 / P2-5 / P2-6**（負債・衛生）

目標: Rapier / launcher の docs・release 残骸を現行（XPBD / HostSupervisor）に合わせ、assets CI が開発用 `_build` ボリュームを共有しないようにし、textured GLB の画像を **寸法先読み → budget → decode** の順にする。

根拠: [improvement-plan-2026-11-12.md](../../0_doc/evaluation/improvement-plan-2026-11-12.md) **P2-3 / P2-5 / P2-6** · [opus-specific-weaknesses-2026-11-12.md](../../0_doc/evaluation/opus/opus-specific-weaknesses-2026-11-12.md)

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| overview / client.md / protocol.md / README / `windows-release.yml` から退役 Rapier・launcher 残骸を除去（履歴 Slice 名は残す） | `launcher.md`（廃止明記）の削除 |
| `ci.sh` assets Docker が開発用 `assets_build` と別 named volume を使う | Elixir 1.19 credo 型検査フレークの根治 |
| 埋め込み画像は宣言寸法が pixel 上限超なら **decode 前に Err** | 単一 `image` decode 中ピークの完全ゼロ化（x0085 非保証を維持） |

---

## Phase

| Phase | 内容 | 状態 |
|:---|:---|:---|
| 0 | Slice + 目次 | **done** |
| 1 | P2-5 docs / release | **done** |
| 2 | P2-6 `ci.sh` assets CI volumes | **done** |
| 3 | P2-3 header dimensions → budget → decode + 否定系 | **done** |
| 4 | improvement-plan / architecture 鮮度 | **done** |

---

## 完了条件

1. 指名 docs / workflow を grep して現行と一致
2. `run_assets_docker` が CI 専用 `_build`（必要なら deps）を上書きマウント
3. 巨大 IHDR・小さな IDAT の tiny GLB が decode 前に budget Err；`cargo test -p client` Green
4. improvement-plan P2-3 / P2-5 / P2-6 を closed 表記

## やらない

- P2-2 catalog / epoch（→ x00d2）
- ~~P2-4 KeyStale~~ → [x00d5](x00d5-keystale-seamless-rejoin.md) / ~~P2-7 ureq~~ → [x00d3](x00d3-relay-agent-ureq-jwks.md) / P2-8 Rooms 分割
- ~~PoP nonce~~ → [x00d4](x00d4-pop-nonce.md)
- `smoke-rapier-host.sh` ファイル名変更（README 説明のみ現行化）

← [目次](../README.md) · [improvement-plan](../../0_doc/evaluation/improvement-plan-2026-11-12.md) · [x00d0](x00d0-desktop-event-command-split.md)
