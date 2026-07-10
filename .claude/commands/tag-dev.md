# dev 分支发版

在 dev 分支提交代码、迭代版本号、打 tag 触发 CI 编译。

## 流程

### 1. 确认状态

```bash
git status
git log --oneline -5
git tag --sort=-v:refname | grep dev | head -3
grep '"version"' src-tauri/tauri.conf.json
```

确认：在 dev 分支、无未暂存的意外文件、当前版本号。

### 2. 暂存并提交代码

按改动性质分 commit（feat/fix/docs/chore），commit 信息用中文。

不要用 `git add -A`，逐个指定文件。

### 3. 迭代版本号

查看最新 dev tag（`git tag --sort=-v:refname | grep dev | head -1`），取最后一位 +1。

tag 格式为 `dev-0.8.X`，tauri.conf.json version 格式为 `0.8.X-dev`。

例如最新 tag 是 `dev-0.8.24`：
- tauri.conf.json version 改为 `0.8.25-dev`
- tag 打 `dev-0.8.25`

修改 tauri.conf.json 后提交：
```bash
git commit -m "chore: 版本号 → 0.8.25-dev"
```

### 4. Push + Tag

```bash
git push origin dev
git tag dev-0.8.25
git push origin dev-0.8.25
```

CI 由 `dev-*` tag 触发，只编译不创建 Release。

### 5. 确认 CI

提示用户检查 GitHub Actions 是否触发。

## 约束

- **禁止在 main 分支打 tag**
- **tag 格式必须是 `dev-0.8.X`（不是 `v0.8.X-dev`）**
- **tauri.conf.json version 必须与 tag 一致（`0.8.X-dev`）**
- **每次 tag 授权是一次性的**（memory: no-tag-without-permission）
- **构建用 `./build.sh`，不要手动 cargo build / npm run build**
