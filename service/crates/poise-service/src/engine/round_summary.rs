//! 把每轮调度的结果整理成记录系统的输入（见 `poise_logbook::Round`），交给日志系统。
//! 只在有重点程序时才交（以及重点程序刚结束的那一轮，让记录系统收尾）：平时不去叫醒日志线程。

use poise_core::prefs::Mode;
use poise_core::schedule::Verdict;
use poise_logbook::{FocusProgram, Round};

use super::{Engine, unix_now_ms};

impl Engine {
    /// 每轮采样（这一轮的结果已经放进 `view` 之后）调用。
    pub(super) fn report_round(&mut self) {
        let Some(view) = self.view.as_ref() else {
            return;
        };
        let was_focused = std::mem::replace(&mut self.round_focused, view.focused);
        if !view.focused && !was_focused {
            return;
        }
        // 玩游戏、界面没显示时不读处理器温度（见 `poise_hardware::SensorLevel`）：手里的是游戏开始前的旧读数，
        // 不记进去。要性能的模式下，大核在忙却被压低了频率也算被限速（不少处理器这时固件的性能上限照样是
        // 100%，见 `poise_core::diagnose::ClockWatch`）；安静、均衡模式下频率低是选的，不算。
        let now = self.now();
        let stale_temp = view.game.is_some() && !self.session.ui_visible(now);
        let wants_speed = matches!(view.effective_mode, Mode::Full | Mode::Vendor);
        let mut round = Round {
            unix_ms: unix_now_ms(),
            focused: view.focused,
            mode: view.effective_mode,
            cpu_pct: self.activity.system_cpu_pct(),
            cpu_temp: view.reading.thermals.cpu_temp.filter(|_| !stale_temp),
            throttled: self.headroom.limited() || (wants_speed && self.clocks.held()),
            ..Default::default()
        };
        if view.focused {
            let domains = self.topology.domains.len();
            for (key, state) in self.scheduler.programs() {
                match state.verdict {
                    Verdict::Playing | Verdict::Focused => {
                        let Some(p) = view.snapshot.find(key.pid).filter(|p| p.facts.key == *key)
                        else {
                            continue;
                        };
                        round.focus.push(FocusProgram {
                            key: *key,
                            name: p.facts.friendly_name(),
                            path: p.facts.image_path.clone(),
                            game: state.verdict == Verdict::Playing,
                            load_pct: state.load_pct,
                            domains: state.cores.domain_mask(domains),
                        });
                    }
                    Verdict::Aside | Verdict::Backseat => round.moved_aside += 1,
                    Verdict::Yielded | Verdict::Lowered => round.yielded.push(*key),
                    Verdict::Normal => {}
                }
            }
        }
        self.logbook.round(round);
    }
}
