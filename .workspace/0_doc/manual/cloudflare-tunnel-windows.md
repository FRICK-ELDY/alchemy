# Cloudflare Tunnel 手順（Windows）

拠点の **auth / world / assets** を、ルーターのポート開放なしで HTTPS 公開する運用手順。  
テンプレ・gitignore: [`edge/cloudflared/`](../../../edge/cloudflared/) · Slice: [x001a](../../roadmap/slices/x001a-https-edge-tunnel.md)

対象 OS: **Windows**（Docker Desktop + ネイティブ `cloudflared`）。  
例ドメイン: `your-domain.com`（自分の Zone に読み替え可）。

## 構成のイメージ

| プロセス | 役割 |
|:---|:---|
| Docker（auth / world / assets） | `127.0.0.1:4002` / `4003` / `4004` で待受 |
| **cloudflared** | 拠点から Cloudflare へ外向き接続し、HTTPS を中継（zenohd と同様、拠点で常時起動） |
| Cloudflare | SSL 終端・DNS・拠点 IP の秘匿 |

Zenoh は本手順の対象外（LAN / localhost。WAN は x001b）。

---

## 初回セットアップ

### 1. cloudflared の導入

`winget` が無い場合は公式 exe を直接置く。

```powershell
$dir = "$env:LOCALAPPDATA\cloudflared"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Invoke-WebRequest `
  -Uri "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe" `
  -OutFile "$dir\cloudflared.exe"
```

**ユーザー環境変数 Path** に次を追加する（システムの「環境変数の編集」GUI から）:

```text
%LOCALAPPDATA%\cloudflared
```

Path を変えたあとは **Cursor / PowerShell を起動し直す**（ターミナルだけ閉じても親プロセスが古い Path のまま）。

```powershell
cloudflared --version
```

### 2. Tunnel 作成

```powershell
cd D:\github\frick\alchemy

cloudflared tunnel login
```

ブラウザで対象 Zone（例: `your-domain.com`）を選んで Authorize。

```powershell
cloudflared tunnel create alchemy-home
cloudflared tunnel list
```

表示された **Tunnel ID（UUID）** を控える。  
credentials は `%USERPROFILE%\.cloudflared\<TUNNEL_ID>.json`。

### 3. credentials と config.yml

`<TUNNEL_ID>` はプレースホルダ。**実際の UUID に置き換える**（文字どおり `<TUNNEL_ID>` のままでは失敗する）。

```powershell
cd D:\github\frick\alchemy

Copy-Item "$env:USERPROFILE\.cloudflared\<TUNNEL_ID>.json" `
  .\edge\cloudflared\credentials.json

Copy-Item .\edge\cloudflared\config.yml.example `
  .\edge\cloudflared\config.yml
```

`edge\cloudflared\config.yml` の例:

```yaml
tunnel: <TUNNEL_ID>
credentials-file: ./credentials.json

ingress:
  - hostname: auth.your-domain.com
    service: http://127.0.0.1:4002
  - hostname: world.your-domain.com
    service: http://127.0.0.1:4003
  - hostname: assets.your-domain.com
    service: http://127.0.0.1:4004
  - service: http_status:404
```

`credentials.json` / `config.yml` / `.env` は **gitignore 済み。リポにコミットしない**。

### 4. DNS（CNAME）

```powershell
cloudflared tunnel route dns alchemy-home auth.your-domain.com
cloudflared tunnel route dns alchemy-home world.your-domain.com
cloudflared tunnel route dns alchemy-home assets.your-domain.com
```

Dashboard → DNS で Proxy（オレンジ雲）オンの CNAME が増えていれば OK。

---

## 毎回の起動

1. Docker Desktop を起動  
2. サービス:

```powershell
cd D:\github\frick\alchemy\auth-server; docker compose up -d
cd D:\github\frick\alchemy\world-server; docker compose up -d
cd D:\github\frick\alchemy\assets-server; docker compose up -d
```

3. ローカル到達（任意）:

```powershell
curl.exe -sf http://127.0.0.1:4002/health
curl.exe -sf http://127.0.0.1:4003/health
curl.exe -sf http://127.0.0.1:4004/health
```

4. Tunnel（**このフォルダから**。ウィンドウは開いたまま）:

```powershell
cd D:\github\frick\alchemy\edge\cloudflared
cloudflared tunnel --config config.yml run
```

成功の目印: `Registered tunnel connection`（`location=nrt…` など）。

QUIC の一時タイムアウトや、connIndex の一部だけ失敗する警告はよくある。  
**少なくとも 1 本** `Registered` が出ていれば公開できる。不安定なら:

```powershell
cloudflared tunnel --config config.yml --protocol http2 run
```

---

## 確認

```powershell
curl.exe -sf https://auth.your-domain.com/health
curl.exe -sf https://world.your-domain.com/health
curl.exe -sf https://assets.your-domain.com/health
```

縦通し（login → Room Token → assets）:

```powershell
cd D:\github\frick\alchemy
$env:AUTH_URL = "https://auth.your-domain.com"
$env:WORLD_URL = "https://world.your-domain.com"
$env:ASSETS_URL = "https://assets.your-domain.com"
bash .scripts/smoke-https-edge.sh
```

デモ成功条件: **別ネットワーク**（例: スマホ回線）から同じ `/health` が取れること。

外クライアント env 例: [`edge/client.env.example`](../../../edge/client.env.example)

---

## トラブルシュート

| 症状 | 見ること |
|:---|:---|
| `cloudflared` が認識されない | Path 未反映 → Cursor ごと再起動。exe の場所を確認 |
| `Copy-Item` で無効な文字 | `<TUNNEL_ID>` を UUID に置換していない |
| credentials-file エラー | `edge\cloudflared` 以外から `run` している |
| Tunnel は動くが 502 | Docker が落ちている / `127.0.0.1:400x` に届かない |
| DNS だけ解決しない | CNAME の Proxy オフ、伝播待ち、レコード名の誤り |
| QUIC timeout が多い | `--protocol http2` を試す |

---

## 保証する / しない

| 保証する | 保証しない |
|:---|:---|
| ポート開放ゼロで 3 サービスへ HTTPS 到達 | Zenoh WAN（→ x001b） |
| 拠点 IP をクライアント設定に書かない | cloudflared HA・複数拠点 |
| 秘密をリポに入れない | WAF / Bot 対策の作り込み |
