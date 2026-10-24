# Roadmap

縦スライス単位の進捗目次。各 Slice の詳細（保証・Phase・起動）は個別ファイルへ。

## 目次

| Slice | 状態 | 要約 | 詳細 |
|:---|:---|:---|:---|
| A | done | ローカル Zenoh 縦スライス（Publish → Subscribe → 描画） | [slice-a.md](slice-a.md) |
| B | done | Room Token 門番（auth JWT 必須） | [slice-b.md](slice-b.md) |
| C-A | done | assets-server 最小（CAS + Quota） | [slice-c-a.md](slice-c-a.md) |
| C-B | done | Coin GLB を wgpu 表示 | [slice-c-b.md](slice-c-b.md) |
| C-C | done | Manifest（低頻度）＋ RenderFrame デルタ（高頻度） | [slice-c-c.md](slice-c-c.md) |
| D | pending | Rapier 最小＋ `is_finite`（内部 60Hz / 配信 30Hz） | [slice-d.md](slice-d.md) |

**現在の次:** [Slice D](slice-d.md)

## 関連

| 文書 | 内容 |
|:---|:---|
| [ローカル CI](ci.md) | `make ci` など |
| [architecture/overview.md](../0_doc/architecture/overview.md) | 目標アーキテクチャ |
| [README.md](../../README.md) | リポジトリ入口・起動手順 |

## 管理ルール

- **1 Slice = 1 ファイル**（保証する／しない・Phase・起動をそのファイルに閉じる）
- 状態が変わったら **この目次の表** と当該 Slice ファイルの両方を更新する
- 新 Slice を切るときは `slice-*.md` を追加し、目次行を足す
- アーキテクチャ正本へのリンクは `../0_doc/architecture/` を使う
