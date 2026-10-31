//! 当前用户：按会话找到登录的用户，"当前用户"的注册表位置、用户目录跟着它（按用户保存的优化项、游戏的
//! 显卡偏好都写在那里）。服务开机就启动，那时可能还没人登录：托盘代理上报会话、界面或托盘连上来时再找。

use poise_core::model::Pid;
use poise_platform::observe::system;
use poise_platform::settings::user::UserContext;

use super::Engine;
use crate::config::RunMode;

impl Engine {
    /// 交互用户变化（登录、切换用户）时，更新"当前用户"的注册表位置。
    /// 没有它，"资源管理器"等按用户保存的优化项无法读写。
    pub(super) fn on_session_changed(&mut self, session_id: u32) {
        if self.cfg.mode == RunMode::Service {
            let user = UserContext::for_session(session_id);
            if user == self.changes.user() {
                return;
            }
            log::info!(
                "交互会话 {session_id}，用户：{:?}",
                user.as_ref().and_then(|u| u.sid.clone())
            );
            self.changes.set_user(user);
            // 服务开机就启动，那时还没人登录，按用户保存的设置（游戏的显卡偏好等）读写不了：现在补上。
            self.resync_game_settings();
            let now = self.now();
            self.power_guard.check_soon(now);
        }
    }

    /// 界面或托盘连上来时还不知道当前用户（服务启动时没找到、托盘代理还没上报）：按它所在的会话找。
    /// 不只靠托盘代理的上报——它没连上时，按用户保存的优化项会一直读不到，显示成没优化。
    pub(super) fn adopt_client_session(&mut self, pid: Pid) {
        if self.cfg.mode != RunMode::Service || self.stopped || self.changes.user().is_some() {
            return;
        }
        // 只认当前在用的那个会话里的（后台断开的会话里的界面不算）。
        if let Some(session_id) = system::session_of(pid)
            && session_id == system::active_console_session()
        {
            self.on_session_changed(session_id);
        }
    }
}
