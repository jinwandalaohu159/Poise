//! 大核、小核此刻实际跑在多少频率，以及大核最高跑到过多少。
//!
//! 整机的平均频率看不出问题：游戏的主线程在一两个大核上，别的核心大多空着。这里按各逻辑处理器的占用加权
//! （空着的核心不算），分大核、小核各算一个。大核正在忙、频率却远低于同样多的大核在忙时跑到过的频率，
//! 说明处理器正被功耗或温度压着——这个比固件的"性能上限"可靠：不少处理器（实测 i7-12700H）功耗墙、
//! 温度墙时那个上限也一直是 100%。
//!
//! 要按"几个线程在忙"分开比：一两个核心忙时能睿频到最高，所有核心一起忙时本来就低得多（全核睿频）。
//! 拿全核负载去比单核的最高频率，编译一跑满就会被当成"被压着"。

/// 参与加权的核心至少要这么忙（空着的核心频率没有意义）。
const COUNTED_LOAD_PCT: f32 = 20.0;
/// 这么忙算"在忙"（报告有几个线程在忙）。
const BUSY_LOAD_PCT: f32 = 50.0;
/// 大核在忙、频率不到最高见过的这个比例：被功耗或温度压着。
const HELD_DOWN_RATIO: f32 = 0.8;

/// 一次读数。没有核心在忙时对应的频率为 None。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ClassClocks {
    /// 大核（不是大小核处理器时是全部核心）按占用加权的频率（MHz）。
    pub performance_mhz: Option<u32>,
    /// 在忙的大核线程数。
    pub performance_busy: u32,
    /// 小核按占用加权的频率（MHz）。
    pub efficiency_mhz: Option<u32>,
}

/// 按占用加权算出大核、小核的频率。三个切片按逻辑处理器编号对应；长度不一致时什么都算不出。
pub fn class_clocks(
    core_mhz: &[u32],
    load_pct: &[f32],
    performance_by_index: &[bool],
) -> ClassClocks {
    let n = core_mhz.len();
    if n == 0 || load_pct.len() != n || performance_by_index.len() != n {
        return ClassClocks::default();
    }
    let weighted = |performance: bool| {
        let (mut sum, mut weight) = (0.0f64, 0.0f64);
        for i in (0..n).filter(|&i| performance_by_index[i] == performance) {
            let (mhz, load) = (core_mhz[i], load_pct[i]);
            if mhz > 0 && load >= COUNTED_LOAD_PCT {
                sum += f64::from(mhz) * f64::from(load);
                weight += f64::from(load);
            }
        }
        (weight > 0.0).then(|| (sum / weight).round() as u32)
    };
    ClassClocks {
        performance_mhz: weighted(true),
        performance_busy: (0..n)
            .filter(|&i| performance_by_index[i] && load_pct[i] >= BUSY_LOAD_PCT)
            .count() as u32,
        efficiency_mhz: weighted(false),
    }
}

/// 大核跑到过多少：按"有几个大核线程在忙"分开记最高的读数（服务运行期间）。
#[derive(Debug, Default)]
pub struct ClockWatch {
    /// 第 n 项：n 个大核线程在忙时见过的最高频率（MHz，0 表示还没见过）。
    peaks: Vec<u32>,
    /// 最近一次读数时是不是被压着。
    held: bool,
}

impl ClockWatch {
    /// 记下一次读数，返回此刻大核是不是被压着：在忙，频率却不到同样多（或更多）线程在忙时见过的最高频率
    /// 的 80%。更多线程在忙时能跑到的，更少时一定也能跑到；同样多的还没见过时不下结论。
    pub fn observe(&mut self, clocks: &ClassClocks) -> bool {
        let busy = clocks.performance_busy as usize;
        self.held = match clocks.performance_mhz {
            Some(mhz) if busy > 0 => {
                if self.peaks.len() <= busy {
                    self.peaks.resize(busy + 1, 0);
                }
                let expected = self.peaks[busy..].iter().copied().max().unwrap_or(0);
                self.peaks[busy] = self.peaks[busy].max(mhz);
                expected > 0 && (mhz as f32) < expected as f32 * HELD_DOWN_RATIO
            }
            _ => false,
        };
        self.held
    }

    /// 最近一次读数时大核是不是被压着（见 [`ClockWatch::observe`]）。
    pub fn held(&self) -> bool {
        self.held
    }

    /// 大核最高见过的频率；还没读到过时为 None。
    pub fn peak_mhz(&self) -> Option<u32> {
        self.peaks.iter().copied().max().filter(|&m| m > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_cores_decide_the_clock_and_idle_ones_do_not_count() {
        // 两个大核、两个小核：大核 0 跑满 4.6 GHz，大核 1 空着（读数没意义）；小核一个半忙。
        let c = class_clocks(
            &[4600, 800, 2400, 0],
            &[100.0, 2.0, 40.0, 0.0],
            &[true, true, false, false],
        );
        assert_eq!(c.performance_mhz, Some(4600));
        assert_eq!(c.performance_busy, 1);
        assert_eq!(c.efficiency_mhz, Some(2400));
        assert_eq!(
            class_clocks(&[4600], &[100.0, 1.0], &[true]),
            ClassClocks::default()
        );
    }

    fn at(mhz: u32, busy: u32) -> ClassClocks {
        ClassClocks {
            performance_mhz: Some(mhz),
            performance_busy: busy,
            efficiency_mhz: None,
        }
    }

    #[test]
    fn a_busy_core_well_below_its_peak_is_held_down() {
        let mut w = ClockWatch::default();
        assert!(!w.observe(&at(4600, 1)));
        assert!(!w.observe(&at(4000, 1)));
        assert!(w.observe(&at(3000, 1)));
        assert_eq!(w.peak_mhz(), Some(4600));
        // 没在忙：低频是正常的。
        assert!(!w.observe(&at(1200, 0)));
    }

    #[test]
    fn all_core_clocks_are_compared_with_all_core_clocks() {
        let mut w = ClockWatch::default();
        assert!(!w.observe(&at(4600, 2)));
        // 所有大核一起忙（编译）：全核睿频本来就低，不算被压着。
        assert!(!w.observe(&at(3400, 12)));
        assert!(!w.observe(&at(3300, 12)));
        // 真的被压着：同样全核在忙，频率掉到见过的八成以下。
        assert!(w.observe(&at(2500, 12)));
        // 线程少一些时，按更多线程在忙时见过的（3.4 GHz）比，还没见过同样多的也能判断。
        assert!(w.observe(&at(2600, 6)));
        assert!(!w.observe(&at(3300, 6)));
    }
}
