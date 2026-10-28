fn main() {
    // 图标与版本信息：任务管理器、托盘设置里显示为 "Poise"。版本号来自 Cargo 工作区。
    println!("cargo:rerun-if-changed=resources/agent.rc");
    println!("cargo:rerun-if-changed=../../../app/Poise.App/Assets/Poise.ico");
    embed_resource::compile("resources/agent.rc", version_defines())
        .manifest_optional()
        .expect("编译资源失败");
}

/// 资源文件里的 POISE_VERSION（1,2,3,0）和 POISE_VERSION_TEXT（"1.2.3"）。
fn version_defines() -> [String; 2] {
    let version = std::env::var("CARGO_PKG_VERSION").expect("缺少版本号");
    let numbers: Vec<&str> = version.split(['.', '-']).take(3).collect();
    [
        format!("POISE_VERSION={},0", numbers.join(",")),
        format!("POISE_VERSION_TEXT=\"{version}\""),
    ]
}
