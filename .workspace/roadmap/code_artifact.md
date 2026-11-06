# Alchemy マルチライセンス分離・適用仕様書 (Cursor Agent 指示書)

## 1. 目的と設計意図（Why）

`FRICK-ELDY/alchemy`（および関連サブモジュール群）は、基本的にはオープンソース（OSS）として広く公開し、個人運営サーバー（Solo Node）や連合（Federation）の普及を促進する方針をとっています。
しかし、「安全性第一」の観点から、単一の寛容なライセンス（MIT等）で全領域を公開するのではなく、**アーキテクチャの責務境界に沿った4階層のライセンス分離**を実施します。

この分離により、以下の4つの安全性を同時に担保します。

1. **エコシステム参入と特許の安全性（`Apache-2.0`）**
   - ワイヤ契約（Protobuf）や中継ルーター規格を明示的な特許許諾・特許報復条項付きの `Apache-2.0` とし、外部ツール開発者やサードパーティの「Zenohリレー貸し屋」が特許リスクやライセンス汚染を恐れずに参入できるようにします。
2. **コア資産の保護とストア配布の安全性（`MPL-2.0`）**
   - 高い開発コストをかけた Pure Rust クライアント／ホストエンジンや Elixir コア基盤が、第三者によって非公開のまま丸パクリ・独自商用化されるのを防ぎます。
   - 同時に、ファイル単位の弱コピーレフトである `MPL-2.0` を採用することで、Steam配信（Steamworks SDKリンク）や将来のコンシューマ／モバイルストア審査、企業導入における法務障壁をクリアします。
   - ※現在のコア4サーバーの暗号・認証機構（Room鍵AEAD、Ephemeral mTLS、RS256 JWT、見る権利と着る権利の分離）は「ケルクホフスの原理」に基づいているため、コードを全面公開してもセキュリティ上のリスクはありません。
3. **盗品回避・チート解析に対する安全性（`Proprietary`）**
   - 暗号とは異なり、「3D特徴量ベクトル（pgvector）による頂点ずらし盗品検知」や「`world-server` (`apps/verifier`) によるホストの抜き打ち改ざん監査（Spot-Check）の具体的な判定閾値・ヒューリスティック」は、コードが公開されると攻撃者に検知回避策を計算されてしまいます。
   - したがって、OSS側には抽象インターフェース（Elixirの `Behaviour` 等）のみを残し、具体的な判定アルゴリズム実装は非公開（Proprietary）の別リポジトリ／プラグインとして隔離します。
4. **経済圏保護とシビル攻撃防止の安全性（`BSL-1.1`）**
   - 個人運営ではOFFにできるオプション拡張の `market-server` および `BillingProvider`（プリペイドコイン台帳・Stripe/MoR連携・売上エスクロー・License Token発行）は `BSL-1.1`（Business Source License 1.1）とします。
   - コードの透明性（監査可能性）を確保しつつ、第三者による商用競合利用を制限することで、悪質業者がスクリプトで海賊版マーケットを大量自動構築するシビル攻撃（Sybil Attack）を構造的に防ぎます。

---

## 2. 4階層ライセンス割り当てマトリクス（What）

| 階層区分 | 対象コンポーネント / ディレクトリ | 適用ライセンス (SPDX) | 公開形態 | Cursor 作業対象 |
| :--- | :--- | :--- | :--- | :--- |
| **① ワイヤ契約・中継規格層** | `protocol`<br>`relay-server`（旧名 `zenoh-router`）<br>`relay-agent` | `Apache-2.0` | 完全OSS | **今回適用** |
| **② 必須コア・クライアント層** | `auth-server`<br>`assets-server`<br>`world-server`<br>`host-server`<br>`client`（HostSupervisor 含む）<br>`shared-sim` (共有クレート) | `MPL-2.0` | 完全OSS | **今回適用** |
| **③ 検疫・監査アルゴリズム層** | ・3D特徴量（pgvector）算出ワーカー<br>・`world-server` (`apps/verifier`) の詳細監査ルール実装 | `LicenseRef-Alchemy-Proprietary` (Proprietary) | 非公開<br>(別リポジトリ / 外部注入) | **境界インターフェースの維持・確認** |
| **④ 決済・商用マーケット層** | `market-server`<br>`BillingProvider` | `BSL-1.1` | Source-Available<br>(商用競合制限) | **スケルトン / 将来作成時の規約登録** |

---

## 3. Cursor Agent への実装タスク指示（How）

以下の Phase 1 〜 Phase 4 の順番で、リポジトリ内のファイル作成・更新を行ってください。
※サブモジュール構成になっている箇所がある場合は、親リポジトリと各サブモジュールの整合性に注意して作業してください。

### Phase 1: ルートディレクトリの整備（マルチライセンス明示と商標保護）

1. **ルート `LICENSE` ファイルの書き換え**
   - リポジトリ直下の `LICENSE` を単一ライセンスの全文から、「マルチライセンス案内（一覧と各ディレクトリへの誘導）」に書き換えてください（テンプレートは第4章を参照）。
2. **`TRADEMARKS.md` の新設**
   - OSSとしてコードを公開しても、「VRAlchemy」「AlchemyEngine」の名称・公式ロゴ・公認バッジはオープンソース許諾の対象外（商標権・ブランド権の留保）であることを明記する `TRADEMARKS.md` をルートに作成してください（フィッシングや公式詐称フォークの防止）。
3. **ルート `README.md` の更新**
   - `README.md` のライセンスセクションを、上記4階層のマルチライセンス構成を説明する表と `LICENSE` / `TRADEMARKS.md` へのリンクに更新してください。

### Phase 2: ワイヤ契約・中継規格層への `Apache-2.0` 適用

1. **対象ディレクトリ**: `protocol/`, `relay-server/`（旧名 `zenoh-router`）, `relay-agent/`（存在する各パッケージ）
2. **`LICENSE` の配置**:
   - 各ディレクトリ直下に `Apache-2.0` の公式ライセンス全文を記載した `LICENSE` ファイルを配置してください。
3. **マニフェストの更新**:
   - `protocol/` や `relay-server/` / `relay-agent/` 内に `Cargo.toml` や `mix.exs`、`package.json` 等が存在する場合、`license = "Apache-2.0"` に更新してください。

### Phase 3: 必須コア・クライアント層への `MPL-2.0` 適用

1. **対象ディレクトリ**:
   - Elixir系: `auth-server/`, `assets-server/`, `world-server/`
   - Rust系: `host-server/`, `client/`, `shared-sim/`（ワークスペース内の各クレート含む）
2. **`LICENSE` の配置**:
   - 各コンポーネントのルートディレクトリ直下に、`MPL-2.0`（Mozilla Public License Version 2.0）の公式全文を記載した `LICENSE` ファイルを配置してください。
3. **Rust マニフェスト（`Cargo.toml`）の更新**:
   - 対象ディレクトリの `[workspace.package]` または `[package]` セクションの `license` フィールドをすべて以下に統一してください。
     ```toml
     license = "MPL-2.0"
     ```
4. **Elixir マニフェスト（`mix.exs`）の更新**:
   - 対象ディレクトリの `project` 定義（Umbrella構成の場合はルートおよび `apps/*/mix.exs`）に以下を設定してください。
     ```elixir
     package: [
       licenses: ["MPL-2.0"]
     ]
     ```

### Phase 4: 検疫・監査層（Proprietary）とマーケット層（BSL-1.1）の境界ルール設定

1. **アーキテクチャ境界の保護（重要）**:
   - `assets-server`（`MPL-2.0`）には「軽量なファイルヘッダ検証」と「検疫ワーカーを呼び出す抽象 `Behaviour`（デフォルトはNo-opまたは基本チェックのみ）」だけを置き、3D特徴量（pgvector類似度）の算出アルゴリズム自体は含めないでください。
   - `world-server` の `apps/verifier`（`MPL-2.0`）には「空間台帳の署名検証」「死活監視フェイルオーバー」「監査エンジンを呼び出す抽象 `Behaviour`」のみを置き、メモリ改ざんを検知する具体的なヒューリスティック閾値・統計アルゴリズムは外部モジュール（`Proprietary`）として注入できる構造にしてください。
2. **`.cursor/rules/` へのライセンス・境界ルールの追加**:
   - 今後の開発でAIエージェントが誤って Proprietary ロジックを MPL-2.0 領域にハードコードしたり、ライセンス表記を崩したりしないよう、`.cursor/rules/license-and-boundaries.mdc`（または既存のルールファイル）に本仕様のルールを追記してください。
3. **`market-server` スケルトンがある場合の対応**:
   - もしリポジトリ内に `market-server` のディレクトリやスケルトンが既に存在する場合は、その直下に `BSL-1.1` 用の `LICENSE` を配置してください。まだディレクトリが存在しない場合は、`.cursor/rules/` への記録のみで構いません。

---

## 4. ファイル生成用テンプレート集

### 4.1 ルート直下 `LICENSE` のテンプレート

```text
Alchemy Platform Multi-License Notice
=====================================

The Alchemy project (including AlchemyEngine and VRAlchemy) is licensed under a
multi-license architecture designed to balance open ecosystem interoperability,
core engine protection, and marketplace security.

Please check the `LICENSE` file in each respective component directory or
submodule for the exact terms that apply to that component:

1. Wire Contracts & Relay Specifications (`Apache-2.0`)
   - Components: `protocol/`, `relay-server/` (formerly `zenoh-router`),
     `relay-agent/`
   - Licensed under the Apache License, Version 2.0.

2. Core Infrastructure & Client Engine (`MPL-2.0`)
   - Components: `auth-server/`, `assets-server/`, `world-server/`,
     `host-server/`, `client/`, `shared-sim/`
   - Licensed under the Mozilla Public License, Version 2.0 (MPL-2.0).

3. Quarantine & Anti-Cheat Heuristics (`Proprietary`)
   - Components: 3D Perceptual Hash / pgvector Quarantine Workers,
     Detailed Spot-Check Verifier Heuristics (maintained in separate private repositories).
   - All rights reserved. Not licensed under an open-source license.

4. Commercial Marketplace & Billing Extensions (`BSL-1.1`)
   - Components: `market-server/`, `BillingProvider`
   - Licensed under the Business Source License 1.1.

Trademarks
----------
Nothing in these licenses grants permission to use the trade names, trademarks,
service marks, or product names of the Licensor ("VRAlchemy", "AlchemyEngine",
and official logos), except as required for reasonable and customary use in
describing the origin of the Work. See `TRADEMARKS.md` for details.
```

### 4.2 ルート直下 `TRADEMARKS.md` のテンプレート

```markdown
# Trademark and Brand Policy

While the core source code of the Alchemy platform is available under open-source licenses (`Apache-2.0` and `MPL-2.0`), the **trademarks, service marks, trade names, and official logos** listed below are **not** licensed under an open-source license.

## Protected Marks
The following names and visual assets are reserved by FRICK-ELDY:
- **AlchemyEngine** / **VRAlchemy**
- Official project logos, icons, and federation verification badges

## Permitted Use
- You may state truthfully that your software or server instance is "based on AlchemyEngine" or "compatible with the Alchemy protocol."
- Unmodified builds created directly from the official source for personal or internal testing may retain the marks.

## Prohibited Use without Prior Written Permission
- Distributing modified forks of `client` or server components under the name "AlchemyEngine" or "VRAlchemy" in a manner that may confuse users into believing it is an official release.
- Using official logos or verification badges to misrepresent an unofficial `market-server` or `world-server` as an officially operated hub.
```

### 4.3 `market-server` 用 `BSL-1.1` ヘッダテンプレート（将来 / スケルトン用）

```text
License: Business Source License 1.1 (BSL-1.1)

Licensor: FRICK-ELDY
Licensed Work: Alchemy Market Server & Billing Provider (`market-server`)

Additional Use Grant:
You may make use of the Licensed Work for non-commercial personal testing and
evaluation purposes. You may NOT use the Licensed Work in a production
marketplace, commercial service, or public federation hub that processes
monetary transactions, prepaid coins, or paid asset license tokens without a
separate commercial agreement from the Licensor.

Change Date: Four (4) years from the date of each release publish.
Change License: Mozilla Public License 2.0 (MPL-2.0)
```

---

## 5. 完了条件（Definition of Done）

- [ ] ルートの `LICENSE`、`TRADEMARKS.md`、`README.md` が4階層ライセンス方針に合わせて更新されていること。
- [ ] `protocol`、`relay-server`（旧名 `zenoh-router`）、`relay-agent` に `Apache-2.0` の `LICENSE` とマニフェスト設定が適用されていること。
- [ ] `auth-server`、`assets-server`、`world-server`、`host-server`、`client` に `MPL-2.0` の `LICENSE` とマニフェスト設定が適用されていること。
- [ ] `.cursor/rules/` に4階層ライセンスと責務境界（Proprietaryロジックの混入防止）のルールが明記されていること。
- [ ] `cargo check` や `mix compile` を実行し、マニフェスト更新による文法エラーやビルド破損が発生していないことを確認すること。