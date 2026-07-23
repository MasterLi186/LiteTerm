# Native 独立构建设计

## 目标

为 `native-prototype/` 提供独立构建入口，确保 Native 开发不会清理、编译或重启当前使用的旧 GuiShell。根目录 `build.sh` 和 `run.sh` 保持字节不变。

## 构建入口

新增 `native-prototype/build.sh`。脚本切换到 `native-prototype/` 目录后依次执行：

1. `cargo build`
2. `cargo clippy --all-targets`
3. `cargo test`
4. 验证 `target/debug/liteterm-native` 为可执行文件

构建使用 Cargo 增量缓存，不删除 `dist/`、`src-tauri/target/` 或旧 GuiShell 进程。当前 Native crate 存在既有 warning，因此 Clippy 以普通模式运行；编译或检查错误仍会使脚本立即失败。

## 启动入口

根目录 `run-native.sh` 继续作为 Native 启动入口。启动前检查：

- Native 二进制不存在或不可执行；
- `native-prototype/Cargo.toml`、`Cargo.lock` 或 `src/` 下任一文件比二进制新。

满足任一条件时，脚本调用 `native-prototype/build.sh`。构建成功后再关闭已有的 `liteterm-native` 进程并启动新二进制；构建失败时不关闭当前进程。

## 隔离与验证

- 根目录 `build.sh` 的 SHA-256 必须保持 `69e913332c3b14111a62f5843cf35dd6751f0f566dc941efc1218364350a2a5e`。
- 根目录 `run.sh` 的 SHA-256 必须保持 `f7d1b7b264548f8e1b968c781c0fa38529a92b24e2bd894b6dcea3b080b1ff6e`。
- Shell 测试使用临时仓库和替身构建器验证缺失产物、过期产物、最新产物及参数透传。
- 完成后真实执行 `native-prototype/build.sh`，确认新二进制包含文件管理器文本，再用 `run-native.sh` 做启动冒烟测试。
