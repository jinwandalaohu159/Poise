//! 一个程序（进程树）此刻的情况：由调度器跨周期判断（见 `judge`），树里的每个进程都按它决定。

use super::policy::ProcessView;
use crate::action::CoreSet;
use crate::classify::Classification;
use crate::model::ProcessKey;

/// 一个程序（进程树）此刻的情况，由调度器跨周期判断；树里的每个进程都按它决定。
#[derive(Debug, Clone, Copy)]
pub(super) struct Program {
    /// 主进程。
    pub(super) root: ProcessKey,
    pub(super) active_game: bool,
    pub(super) gaming: bool,
    /// 是重点程序时，它的核心范围。
    pub(super) focus: Option<CoreSet>,
    /// 有重点程序（或刚结束不久）。
    pub(super) focusing: bool,
    /// 让到小核时分到的小核范围（见 `placement`）。
    pub(super) efficiency: Option<CoreSet>,
    pub(super) recently_foreground: bool,
    pub(super) boosted: bool,
    pub(super) audible: bool,
    /// 主进程自己把优先级调到了"高"或更高。
    pub(super) self_elevated: bool,
    pub(super) busy: bool,
    pub(super) busy_with_audio: bool,
    /// 刚停止使用不久（见 `BusyRule::reyield_after_ms`）。
    pub(super) settling: bool,
    /// 让路时完整让路（见 `Desired::YIELDED`），否则只降优先级。
    pub(super) yield_fully: bool,
}

impl Program {
    /// 树里某个进程的情况：程序的情况加上它自己是否在输出声音。
    pub(super) fn view<'a>(&self, class: &'a Classification, emitting: bool) -> ProcessView<'a> {
        ProcessView {
            class,
            active_game: self.active_game,
            gaming: self.gaming,
            focus: self.focus,
            focusing: self.focusing,
            recently_foreground: self.recently_foreground,
            boosted: self.boosted,
            audible: self.audible,
            emitting,
            self_elevated: self.self_elevated,
            busy: self.busy,
            busy_with_audio: self.busy_with_audio,
            settling: self.settling,
            yield_fully: self.yield_fully,
        }
    }
}
