fn main() {
    // 图标、版本信息与清单（通用控件 6.0：进度对话框需要；按显示器缩放；以调用者身份运行）。
    println!("cargo:rerun-if-changed=resources/updater.rc");
    println!("cargo:rerun-if-changed=resources/updater.manifest");
    println!("cargo:rerun-if-changed=../../../app/Poise.App/Assets/Poise.ico");
    embed_resource::compile("resources/updater.rc", version_defines())
        .manifest_required()
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
