fn main() {
    // 图标与版本信息：任务管理器、托盘设置里显示为 "Poise"。
    println!("cargo:rerun-if-changed=resources/agent.rc");
    println!("cargo:rerun-if-changed=../../../app/Poise.App/Assets/Poise.ico");
    embed_resource::compile("resources/agent.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("编译资源失败");
}
