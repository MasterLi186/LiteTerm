# Native-only 开发约束

LiteTerm 当前唯一开发、构建和运行目标是 `native-prototype/`。

- 默认只修改 `native-prototype/` 中的产品代码。
- 允许按需修改 `run-native.sh`、`native-prototype/build.sh` 和直接描述 Native 版本的文档。
- `src/`、`src-tauri/` 与根目录旧 GTK Rust 实现均为历史参考，禁止修改。
- 只有用户明确点名其他实现时，才能临时修改对应目录；授权仅限当次明确范围。
- Native 版本默认使用 `cargo build --manifest-path native-prototype/Cargo.toml` 构建，使用 `./run-native.sh` 启动。`./native-prototype/build.sh` 会执行自动测试，仅在用户明确授权测试时使用。

旧设计文档若仍描述 React/Tauri/GTK，应视为历史资料，不得据此选择实现入口。
