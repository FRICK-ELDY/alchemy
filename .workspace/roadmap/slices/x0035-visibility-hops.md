# x0035 — friends_plus / followers / followers_plus

> **ID:** `x0035` · **slug:** `visibility-hops` · **status:** `done` · **area:** `[auth, world, docs]` · **depends_on:** `[x0034]`

目標: 2 hop 公開範囲の判定を auth Access に実装し、world Token 門番から利用する。

| 段階 | 資格 |
|:---|:---|
| `friends_plus` | Friends ∪ フレンドの確定フレンド |
| `followers` | owner をフォローしている人 |
| `followers_plus` | Followers ∪ そのフォロワーをフォローしている人 |

## 成功条件

1. FoF は friends_plus 可・friends 不可
2. follow-of-follower は followers_plus 可・followers 不可
3. 共有テスト（auth `SocialTest`）緑

← [目次](../README.md)
