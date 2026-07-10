# dev 分支发版

在 dev 分支提交代码、迭代版本号、打 tag 触发 CI 编译。

## 流程

### 1. 确认状态

```bash
git status
git log --oneline -5
git tag --sort=-v:refname | head -3
grep '"version"' src-tauri/tauri.conf.json
```

确认：在 dev 分支、无未暂存的意外文件、当前版本号。

### 2. 暂存并提交代码

按改动性质分 commit（feat/fix/docs/chore），commit 信息用中文。

不要用 `git add -A`，逐个指定文件。

### 3. 迭代版本号

读取 `src-tauri/tauri.conf.json` 的 `version` 字段（格式 `X.Y.Z-dev`），将最后一位 +1。

```
0.8.16-dev → 0.8.17-dev
```

修改后提交：
```bash
git commit -m "chore: 版本号 X.Y.Z-dev → X.Y.(Z+1)-dev"
```

### 4. Push + Tag

```bash
git push origin dev
git tag vX.Y.(Z+1)-dev
git push origin vX.Y.(Z+1)-dev
```

tag 格式与 version 字段一致，加 `v` 前缀。CI 由 `v*` tag 触发。

### 5. 确认 CI

提示用户检查 GitHub Actions 是否触发。

## 约束

- **禁止在 main 分支打 tag**
- **版本号必须与 tauri.conf.json 一致**（memory: sync-version-before-tag）
- **每次 tag 授权是一次性的**（memory: no-tag-without-permission）
- **构建用 `./build.sh`，不要手动 cargo build / npm run build**
