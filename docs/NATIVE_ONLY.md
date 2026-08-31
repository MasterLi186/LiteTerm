# Native-only 开发约束

LiteTerm 当前唯一开发、构建和运行目标是 `native-prototype/`。

- 默认只修改 `native-prototype/` 中的产品代码。
- 允许按需修改 `run-native.sh`、`native-prototype/build.sh` 和直接描述 Native 版本的文档。
- `src/`、`src-tauri/` 与根目录旧 GTK Rust 实现均为历史参考，禁止修改。
- 只有用户明确点名其他实现时，才能临时修改对应目录；授权仅限当次明确范围。
- 日常开发从 `native-prototype/` 执行 `cargo build --locked --bin liteterm-native` 构建 debug Native 版本，回到仓库根目录后使用 `./run-native.sh` 启动。若 debug 二进制缺失或过期，运行入口会直接执行该 cargo 构建，不会生成 `.deb`。
- `native-prototype/build.sh` 是无参数的本地 Debian 发布打包入口：执行 locked release 构建并生成 `native-prototype/dist/installers/LiteTerm-Native_<版本>_<架构>.deb`；首次缺少时自动安装固定的 `cargo-packager 0.11.8`。该脚本只打包、不运行自动测试，也不会自动 `sudo` 安装。
- 未经用户明确授权，禁止运行 `cargo test`、`cargo test --no-run`、测试脚本或 `cargo clippy --all-targets`（以及会执行或编译自动测试的命令）。

旧设计文档若仍描述 React/Tauri/GTK，应视为历史资料，不得据此选择实现入口。
