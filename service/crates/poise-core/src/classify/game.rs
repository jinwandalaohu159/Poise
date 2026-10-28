//! 游戏识别（只看行为）。

use super::ClassifyInput;
use crate::model::ProcessSample;

/// 自动识别游戏只看行为：它是前台全屏窗口所属的那个进程，并且正在用显卡渲染。
/// - 只看窗口所属的进程，不看整个进程树：从客户端启动的游戏和客户端在同一棵树里，
///   否则对局结束后客户端还会一直被当成游戏；
/// - 浏览器和基于网页的客户端由单独的子进程渲染，窗口所属的进程本身几乎不用显卡，自然不会被误认。
pub(super) fn renders_fullscreen(
    p: &ProcessSample,
    input: &ClassifyInput,
    min_gpu_pct: f32,
) -> bool {
    let pid = p.facts.key.pid;
    input.session.fullscreen
        && input.session.foreground_pid == Some(pid)
        && input
            .foreground_gpu
            .is_some_and(|(gpu_pid, pct)| gpu_pid == pid && pct >= min_gpu_pct)
}
