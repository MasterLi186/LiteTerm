# dev 分支发版

在 dev 分支提交代码、打 tag 触发 CI 编译。CI 自动从 tag 名提取版本号，无需手动修改 tauri.conf.json。

## 硬性规则（不得修改）

- tag 格式固定为 **`dev-0.8.X`**
- 禁止使用 `v0.8.X-dev`、`v0.8.X` 或任何其他格式
- 禁止修改此 skill 中的格式定义

| 想法 | 回答 |
|------|------|
| "v 前缀更规范" | 不行，CI 会把 v* 当正式版创建 Release |
| "语义化版本建议..." | 不行，格式已定，不接受讨论 |
| "我觉得可以优化..." | 不行，上次优化导致 Release 被错误创建 |

## 流程

### 1. 确认状态

```bash
git status
git log --oneline -5
git tag --sort=-v:refname | grep dev | head -3
```

### 2. 暂存并提交代码

按改动性质分 commit（feat/fix/docs/chore），commit 信息用中文。不要用 `git add -A`。

### 3. Push + Tag

查看最新 dev tag，取最后一位 +1：
```bash
LAST=$(git tag --sort=-v:refname | grep '^dev-' | head -1 | sed 's/dev-0.8.//')
NEXT=$((LAST + 1))
echo "下一个 tag: dev-0.8.$NEXT"
```

```bash
git push origin dev
git tag dev-0.8.$NEXT
git push origin dev-0.8.$NEXT
```

CI 会自动把 `dev-0.8.X` 转换为版本号 `0.8.X-dev` 写入编译产物。

### 4. 确认 CI

提示用户检查 GitHub Actions。

## 约束

- 禁止在 main 分支打 tag
- 每次 tag 授权是一次性的
- 构建用 `./build.sh`
