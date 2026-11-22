fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=assets/icons/icon.ico");

    // Windows：把图标和版本信息嵌入 exe 资源，资源管理器里的 exe 文件才有 LiteTerm 图标
    // （window_icon.rs 只在运行时设置窗口图标，exe 文件本身原来没有图标）。
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icons/icon.ico");
        // 任务管理器的进程名取自 FileDescription，默认会是 Cargo.toml 里的长描述
        resource.set("FileDescription", "LiteTerm");
        resource.set("ProductName", "LiteTerm");
        if let Err(error) = resource.compile() {
            panic!("嵌入 Windows 图标资源失败：{error}");
        }
    }
}
