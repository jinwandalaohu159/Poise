//! "自动"模式：按负载、空闲、供电在安静 / 均衡 / 全力之间切换。
//!
//! - 持续高负载（前台程序或整机）且插着电 → 全力；
//! - 用户长时间不操作且整机空闲 → 安静；
//! - 其余 → 均衡；用电池时不进全力。
//!
//! 升档要持续一段时间（短暂的峰值不切换），降档等得更久（避免来回跳）；拔掉电源立即生效。
//!
//! 不按温度切换：处理器和显卡到了温度上限会自己降频保护，Poise 再按温度切模式只会和硬件
//! 抢方向盘——每次切换散热模式都卡一下，降到低功耗模式又白白损失性能，温度一降又切回来，反复循环。

use crate::model::Millis;
use crate::prefs::Mode;

#[derive(Debug, Clone, Copy)]
pub struct AutoModeConfig {
    /// 升档前目标需要保持的时间。
    pub up_after_ms: Millis,
    /// 降档前目标需要保持的时间。
    pub down_after_ms: Millis,
    /// 用户多久不操作算空闲。
    pub idle_after_ms: Millis,
    /// 空闲时整机 CPU 不超过这个比例才进入安静。
    pub idle_cpu_pct: f32,
    /// 前台程序（整个进程树）占用超过这么多个核心算高负载。
    pub heavy_foreground_cores: f32,
    /// 整机 CPU 超过这个比例算高负载（后台渲染、编译等）。
    pub heavy_system_pct: f32,
}

impl Default for AutoModeConfig {
    fn default() -> Self {
        Self {
            up_after_ms: 20_000,
            down_after_ms: 120_000,
            idle_after_ms: 5 * 60_000,
            idle_cpu_pct: 10.0,
            heavy_foreground_cores: 2.0,
            heavy_system_pct: 60.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AutoInput {
    pub now: Millis,
    pub on_battery: bool,
    /// 用户多久没有操作键盘鼠标。
    pub user_idle_ms: Millis,
    /// 整机 CPU 占用（0～100）。
    pub system_cpu_pct: f32,
    /// 前台程序（整个进程树）占用的核心数。
    pub foreground_cores: f32,
}

#[derive(Debug)]
pub struct AutoMode {
    cfg: AutoModeConfig,
    current: Mode,
    /// 正在等待生效的目标与开始时间。
    pending: Option<(Mode, Millis)>,
}

impl Default for AutoMode {
    fn default() -> Self {
        Self::new(AutoModeConfig::default())
    }
}

impl AutoMode {
    pub fn new(cfg: AutoModeConfig) -> Self {
        Self {
            cfg,
            current: Mode::Balanced,
            pending: None,
        }
    }

    /// 当前选定的模式（安静、均衡或全力）。
    pub fn current(&self) -> Mode {
        self.current
    }

    /// 每个采样周期调用一次，返回此刻应使用的模式。
    pub fn decide(&mut self, input: &AutoInput) -> Mode {
        let target = self.target(input);
        if target == self.current {
            self.pending = None;
            return self.current;
        }
        if target.rank() < self.current.rank() && self.limit(input).rank() < self.current.rank() {
            // 拔掉电源：立即降下来。
            self.switch(target);
            return self.current;
        }
        let since = match self.pending {
            Some((mode, since)) if mode == target => since,
            _ => {
                self.pending = Some((target, input.now));
                input.now
            }
        };
        let wait = if target.rank() > self.current.rank() {
            self.cfg.up_after_ms
        } else {
            self.cfg.down_after_ms
        };
        if input.now.saturating_sub(since) >= wait {
            self.switch(target);
        }
        self.current
    }

    fn switch(&mut self, mode: Mode) {
        self.current = mode;
        self.pending = None;
    }

    /// 只看负载的理想模式，再受供电限制。
    fn target(&self, input: &AutoInput) -> Mode {
        let cfg = &self.cfg;
        let heavy = input.foreground_cores >= cfg.heavy_foreground_cores
            || input.system_cpu_pct >= cfg.heavy_system_pct;
        let idle =
            input.user_idle_ms >= cfg.idle_after_ms && input.system_cpu_pct < cfg.idle_cpu_pct;
        let wanted = if heavy {
            Mode::Full
        } else if idle {
            Mode::Quiet
        } else {
            Mode::Balanced
        };
        let limit = self.limit(input);
        if wanted.rank() > limit.rank() {
            limit
        } else {
            wanted
        }
    }

    /// 供电允许的最高模式。
    fn limit(&self, input: &AutoInput) -> Mode {
        if input.on_battery {
            Mode::Balanced
        } else {
            Mode::Full
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Millis = 1_000;

    fn input(now: Millis) -> AutoInput {
        AutoInput {
            now,
            on_battery: false,
            user_idle_ms: 0,
            system_cpu_pct: 15.0,
            foreground_cores: 0.5,
        }
    }

    fn heavy(now: Millis) -> AutoInput {
        AutoInput {
            foreground_cores: 4.0,
            ..input(now)
        }
    }

    #[test]
    fn sustained_load_goes_full_but_a_spike_does_not() {
        let mut a = AutoMode::default();
        assert_eq!(a.decide(&heavy(0)), Mode::Balanced);
        // 10 秒后负载消失：不切换。
        assert_eq!(a.decide(&input(10 * S)), Mode::Balanced);
        assert_eq!(a.decide(&heavy(12 * S)), Mode::Balanced);
        assert_eq!(a.decide(&heavy(32 * S)), Mode::Full);
    }

    #[test]
    fn coming_down_waits_longer_than_going_up() {
        let mut a = AutoMode::default();
        a.decide(&heavy(0));
        a.decide(&heavy(20 * S));
        assert_eq!(a.current(), Mode::Full);
        assert_eq!(a.decide(&input(30 * S)), Mode::Full);
        assert_eq!(a.decide(&input(100 * S)), Mode::Full);
        assert_eq!(a.decide(&input(150 * S)), Mode::Balanced);
    }

    #[test]
    fn long_idle_goes_quiet() {
        let mut a = AutoMode::default();
        let idle = |now| AutoInput {
            user_idle_ms: 6 * 60 * S,
            system_cpu_pct: 3.0,
            ..input(now)
        };
        a.decide(&idle(0));
        assert_eq!(a.decide(&idle(130 * S)), Mode::Quiet);
        // 用户回来：升到均衡也要稳定一会儿。
        assert_eq!(a.decide(&input(131 * S)), Mode::Quiet);
        assert_eq!(a.decide(&input(152 * S)), Mode::Balanced);
    }

    #[test]
    fn never_full_on_battery() {
        let mut a = AutoMode::default();
        let on_battery = |now| AutoInput {
            on_battery: true,
            ..heavy(now)
        };
        a.decide(&on_battery(0));
        assert_eq!(a.decide(&on_battery(60 * S)), Mode::Balanced);
    }
}
