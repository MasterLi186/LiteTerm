# Native-only 开发约束

LiteTerm 当前唯一开发、构建和运行目标是仓库根目录的 LiteTerm Native 实现（Rust + winit + wgpu + egui，无 WebView）。

- 产品代码位于根目录 `src/`；`assets/`、`scripts/`、`packaging/`、`Cargo.toml` 也在根目录。
- 不要再创建 `native-prototype/` 子目录，也不要把旧 React/Tauri/GTK 代码重新引入本分支。
- 历史实现只保留在 `legacy-tauri` 分支和旧 tag 中，仅作参考。
- 日常开发在仓库根目录执行 `cargo build --locked --bin liteterm-native` 构建 debug 版本，使用 `./run-native.sh` 启动。若 debug 二进制缺失或过期，运行入口会执行该 cargo 构建，不会生成 `.deb`。
- `./build.sh` 是无参数的本地 Debian 发布打包入口：执行 locked release 构建并生成 `dist/installers/LiteTerm-Native_<版本>_<架构>.deb`；首次缺少时自动安装固定的 `cargo-packager 0.11.8`。该脚本只打包、不运行自动测试，也不会自动 `sudo` 安装。
- 主题生成源是 `assets/themes.ts`，生成文件是 `src/themes_generated.rs`；使用 `node scripts/generate-themes.mjs` 重新生成，禁止手改生成文件。
- 未经用户明确授权，禁止运行 `cargo test`、`cargo test --no-run`、测试脚本或 `cargo clippy --all-targets`（以及会执行或编译自动测试的命令）。

`docs/` 下仍描述 React/Tauri/GTK 的旧设计文档属于历史资料，不得据此选择实现入口。
