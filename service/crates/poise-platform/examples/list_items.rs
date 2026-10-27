//! 只读：列出本机的动态优化项（开发时核对用）。
use poise_core::optimize::Text;
use poise_platform::settings::{dynamic_items, user::UserContext};

fn main() {
    let user = UserContext::current_process_user();
    for i in dynamic_items::context_menu_items(user.as_ref())
        .into_iter()
        .chain(dynamic_items::navigation_items(user.as_ref()))
        .chain(dynamic_items::startup_items(user.as_ref()))
    {
        let t = match i.title {
            Text::Plain(s) | Text::Key(s) => s,
        };
        let d = match i.description {
            Text::Plain(s) | Text::Key(s) => s,
        };
        println!("{:?}\t{}\t{}\tslow={}", i.group, t, d, i.slow_hint);
    }
}
