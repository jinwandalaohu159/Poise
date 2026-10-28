fn main() {
    // 图标与版本信息：任务管理器的"服务"、文件属性里显示为 "Poise 服务"。
    println!("cargo:rerun-if-changed=resources/service.rc");
    println!("cargo:rerun-if-changed=../../../app/Poise.App/Assets/Poise.ico");
    embed_resource::compile("resources/service.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("编译资源失败");
}
