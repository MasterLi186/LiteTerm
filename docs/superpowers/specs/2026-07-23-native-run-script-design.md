# Native 启动脚本设计

## 目标

在仓库根目录新增 `run-native.sh`，用于启动 `native-prototype/target/debug/liteterm-native`。现有 `run.sh` 继续专门启动旧的 `src-tauri/target/debug/guishell-tauri`，内容和行为均不修改。

## 行为

`run-native.sh` 使用 Bash 并在执行前切换到脚本所在的仓库根目录，确保从任意工作目录调用都能正确解析二进制路径。

脚本按以下顺序执行：

1. 检查 `native-prototype/target/debug/liteterm-native` 是否为可执行文件；若不存在，输出中文提示并以非零状态退出。
2. 查找已有的 `liteterm-native` 进程；若存在，仅终止这些 native 进程，不匹配或影响 `guishell-tauri`。
3. 输出简短启动提示。
4. 使用 `exec` 启动 native 二进制，并将传给脚本的全部参数原样转发。

脚本不设置 WebKitGTK 环境变量，因为 native 客户端不使用 WebView；也不调用 Cargo 或构建命令。缺少产物时提示用户先完成 native 构建。

## 验证

- 比较 `run.sh` 修改前后的校验值，确认旧脚本未变化。
- 使用临时替身二进制验证参数透传、工作目录和退出状态。
- 验证缺少二进制时返回非零状态且错误信息清晰。
- 对 `run-native.sh` 执行 `bash -n`，并确认文件具有可执行权限。
