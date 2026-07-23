# Native 半透明配色修复设计

## 目标

修复 LiteTerm Native 中半透明颜色过曝的问题，使侧边栏、进程列表、磁盘列表、网络图和标签栏的视觉效果与 `main` 分支 React 界面保持一致。此次只修正颜色构造方式，不调整布局、字号、基础主题色或透明度数值。

## 根因

Native 当前将 CSS 风格的未预乘 RGBA 数值传给了
`egui::Color32::from_rgba_premultiplied`。例如白色隔行背景使用
`(255, 255, 255, 4)`；该接口要求 RGB 已经按 Alpha 预乘，因此渲染结果接近白色，而不是 `main` 中的 `rgba(255,255,255,0.015)`。

## 实现范围

将 `native-prototype/src/` 中现有的
`Color32::from_rgba_premultiplied` 调用改为
`Color32::from_rgba_unmultiplied`，覆盖：

- `sidebar.rs`：连接悬停、进程标签、进程和磁盘隔行背景、CPU 使用率填充、网络图填充；
- `tab_bar.rs`：标签悬停和标签操作区背景。

保留现有 RGB 和 Alpha 参数，使其直接对应 `main` 的 CSS RGBA 值。此次不引入独立主题模块，也不修改不透明的 `from_rgb` 调用。

## 开发顺序

1. 先完成并验证 Native 独立构建脚本及 `run-native.sh` 自动重建；
2. 再增加配色回归检查并修正全部错误调用；
3. 使用新的 `native-prototype/build.sh` 完成真实构建、Clippy 和测试；
4. 启动 Native 进行截图对比。

根目录旧 `build.sh` 和 `run.sh` 必须保持字节不变。

## 验证

自动检查 Native 业务源码中不再出现
`Color32::from_rgba_premultiplied`，并通过 Rust 编译、Clippy 和测试。视觉验收重点检查进程/磁盘隔行背景不再发白、CPU 徽标不再过曝、网络填充和标签悬停保持低透明度。
