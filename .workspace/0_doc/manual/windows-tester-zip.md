# Windows テスター ZIP（配布）

> **正本公開**: タグ push → [`.github/workflows/windows-release.yml`](../../../.github/workflows/windows-release.yml) が梱包して Pre-release を作成  
> **ローカル入口**: `make pack-windows` / `make publish-windows`  
> **スクリプト**: [`.scripts/pack-windows-zip.sh`](../../../.scripts/pack-windows-zip.sh) · [`.scripts/publish-windows-release.sh`](../../../.scripts/publish-windows-release.sh)  
> **公開先**: https://github.com/FRICK-ELDY/alchemy/releases （Pre-release）

## 保証 / 非保証

| 保証する | 保証しない |
|:---|:---|
| `v0.0.0-dev.YYYYMMDDHHMM`（JST）形式の版名 | SemVer 互換約束（`v0.1.0` まで） |
| CI が上げた ZIP に launcher / host-server / client の release exe | auth / world / assets / zenohd の同梱 |
| GitHub Pre-release（`--latest=false`）は **CI のみ**が作成 | ローカル `gh release`（レース防止のため廃止） |
| `build-info.json` に version / gitSha / builtAt / **rustcHost** | SmartScreen・インストーラ・自動更新 |
| | assets 不通時の Coin fixture フォールバック（ZIP 外・ビルドマシンパス） |

## 版付け

- タグ・ZIP 名: `v0.0.0-dev.YYYYMMDDHHMM`（例 `v0.0.0-dev.202610281915`）
- タイムゾーン: **JST**
- 同一分の二重タグは禁止（衝突時は分を進めるか `.2` を付ける）

## 公開経路（一本化）

```text
git tag v0.0.0-dev.…  →  push tag  →  windows-release.yml  →  Pre-release ZIP
```

ローカル `publish-windows` は **タグ作成と push だけ**する。`gh release create/upload` は呼ばない（CI とレースしない）。

## 実行環境

`pack-windows` は **ネイティブ Windows**（Git Bash）または **windows-latest CI** 専用。  
CI は `make` を使わず `.scripts/pack-windows-zip.sh` を直接実行する（MinGW make → WSL bash 問題を避ける）。  
WSL からはタグ push のみ。

## ローカルで梱包（Windows + Git Bash・任意の煙）

```bash
make pack-windows
VERSION=v0.0.0-dev.202610281915 make pack-windows
SKIP_BUILD=1 make pack-windows
```

成果物は `dist/` に残るが、**Releases への正本アップロードはしない**。

## タグを切って CI に公開させる

作業ツリーをクリーンにして（WSL 可）:

```bash
make publish-windows CREATE_TAG=1 PUSH_TAG=1
# 既定 SKIP_PACK=1。ネイティブで煙を見たいときだけ:
# SKIP_PACK=0 make publish-windows CREATE_TAG=1 PUSH_TAG=1
```

手動でも可:

```bash
TAG="v0.0.0-dev.$(TZ=Asia/Tokyo date +%Y%m%d%H%M)"
git tag -a "$TAG" -m "Alchemy Windows tester $TAG"
git push origin "refs/tags/$TAG"
```

## テスター前提（要約）

1. Releases の最新 Pre-release から ZIP を取得して展開
2. VC++ x64 再頒布が無い環境では起動に失敗しうる（README-testers.txt 参照）
3. `env.testers.example.cmd` → `env.local.cmd` に URL / `ZENOH_CONNECT` を設定（着信ポート開放は不要）
4. `Start.cmd` を実行（失敗時は pause）
5. assets 不通時のローカル Coin fixture は **壊れてもよい**（将来同梱・埋め込みは別スライス）
