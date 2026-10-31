//! 把每轮的摘要（见 [`Round`]）汇总成一段一段的记录。纯计算，不读系统、不写盘。
//!
//! 一段从出现重点程序开始，到调度不再保持重点程序为止（两局游戏之间的短暂间隙算同一段）；
//! 结束时间是最后一次有重点程序的时刻。太短的不记。两轮之间隔了很久（电脑睡眠过）时，之前的一段到此为止，
//! 醒来后的算新的一段。

use std::collections::HashSet;

use poise_core::model::{Millis, ProcessKey};
use poise_protocol::{RecordProgram, SessionRecord};

use crate::round::Round;

/// 一条记录最多列出几个重点程序（按作为重点程序的时长排序）。
const MAX_PROGRAMS: usize = 5;
/// 短于这个时长的不记（一次短暂的编译、打开一个大文件）。
const MIN_SESSION_MS: Millis = 30_000;
/// 两轮之间的间隔最多按这么久算（服务卡住时不把空白算进时长）。
const MAX_STEP_MS: Millis = 10_000;
/// 两轮之间隔了这么久：电脑睡眠过，之前的一段结束。
const GAP_SPLITS_MS: u64 = 60_000;

#[derive(Debug, Default)]
pub(crate) struct SessionRecorder {
    open: Option<Open>,
}

impl SessionRecorder {
    /// 喂一轮。一段刚结束（并且够长）时返回它的记录。
    pub fn observe(&mut self, round: &Round) -> Option<SessionRecord> {
        if !round.focused {
            return self.finish();
        }
        let slept = self
            .open
            .as_ref()
            .is_some_and(|o| round.unix_ms.saturating_sub(o.last_at) > GAP_SPLITS_MS);
        let ended = if slept { self.finish() } else { None };
        if self.open.is_none() {
            if round.focus.is_empty() {
                return ended;
            }
            self.open = Some(Open::start(round.unix_ms));
        }
        if let Some(open) = self.open.as_mut() {
            open.add(round);
        }
        ended
    }

    /// 结束正在记录的一段（Poise 暂停、服务停止时）。够长时返回它的记录。
    pub fn finish(&mut self) -> Option<SessionRecord> {
        let record = self.open.take()?.close();
        (record.ended_unix_ms.saturating_sub(record.started_unix_ms) >= MIN_SESSION_MS)
            .then_some(record)
    }
}

/// 正在记录的一段。
#[derive(Debug)]
struct Open {
    record: SessionRecord,
    /// 与 `record.programs` 一一对应：程序，以及它作为重点程序的累计毫秒数（按毫秒累计，
    /// 每轮的零头不会被舍掉）。
    keys: Vec<(ProcessKey, Millis)>,
    yielded: HashSet<ProcessKey>,
    last_at: u64,
    cpu_sum: f64,
    samples: u32,
    temp_sum: f64,
    temp_samples: u32,
    /// 被限速的累计毫秒数。
    throttled_ms: Millis,
}

impl Open {
    fn start(unix_ms: u64) -> Self {
        Self {
            record: SessionRecord {
                started_unix_ms: unix_ms,
                ended_unix_ms: unix_ms,
                programs: Vec::new(),
                modes: Vec::new(),
                moved_aside_peak: 0,
                yielded_programs: 0,
                cpu_pct_avg: 0.0,
                cpu_pct_peak: 0.0,
                cpu_temp_avg: None,
                cpu_temp_peak: None,
                throttled_secs: 0,
            },
            keys: Vec::new(),
            yielded: HashSet::new(),
            last_at: unix_ms,
            cpu_sum: 0.0,
            samples: 0,
            temp_sum: 0.0,
            temp_samples: 0,
            throttled_ms: 0,
        }
    }

    fn add(&mut self, round: &Round) {
        let step = round.unix_ms.saturating_sub(self.last_at).min(MAX_STEP_MS);
        self.last_at = round.unix_ms;
        let r = &mut self.record;
        if !round.focus.is_empty() {
            r.ended_unix_ms = round.unix_ms;
        }
        for f in &round.focus {
            let i = match self.keys.iter().position(|(k, _)| *k == f.key) {
                Some(i) => i,
                None => {
                    self.keys.push((f.key, 0));
                    r.programs.push(RecordProgram {
                        name: f.name.clone(),
                        path: f.path.clone(),
                        game: false,
                        focused_secs: 0,
                        peak_load_pct: 0,
                        widest_domains: 0,
                    });
                    r.programs.len() - 1
                }
            };
            self.keys[i].1 += step;
            let p = &mut r.programs[i];
            p.game |= f.game;
            p.focused_secs = (self.keys[i].1 / 1000) as u32;
            p.peak_load_pct = p.peak_load_pct.max(f.load_pct);
            if f.domains.count_ones() > p.widest_domains.count_ones() {
                p.widest_domains = f.domains;
            }
        }
        r.moved_aside_peak = r.moved_aside_peak.max(round.moved_aside);
        self.yielded.extend(round.yielded.iter().copied());
        if !r.modes.contains(&round.mode) {
            r.modes.push(round.mode);
        }
        self.cpu_sum += f64::from(round.cpu_pct);
        self.samples += 1;
        r.cpu_pct_peak = r.cpu_pct_peak.max(round.cpu_pct);
        if let Some(t) = round.cpu_temp {
            self.temp_sum += f64::from(t);
            self.temp_samples += 1;
            r.cpu_temp_peak = Some(r.cpu_temp_peak.map_or(t, |p| p.max(t)));
        }
        if round.throttled {
            self.throttled_ms += step;
        }
    }

    fn close(mut self) -> SessionRecord {
        let r = &mut self.record;
        r.yielded_programs = self.yielded.len() as u32;
        if self.samples > 0 {
            r.cpu_pct_avg = (self.cpu_sum / f64::from(self.samples)) as f32;
        }
        if self.temp_samples > 0 {
            r.cpu_temp_avg = Some((self.temp_sum / f64::from(self.temp_samples)) as f32);
        }
        r.throttled_secs = (self.throttled_ms / 1000) as u32;
        r.programs
            .sort_by_key(|p| std::cmp::Reverse(p.focused_secs));
        r.programs.truncate(MAX_PROGRAMS);
        self.record
    }
}

#[cfg(test)]
mod tests {
    use poise_core::prefs::Mode;

    use super::*;
    use crate::round::FocusProgram;

    const GAME: ProcessKey = ProcessKey { pid: 7, created: 1 };
    const SYNC: ProcessKey = ProcessKey { pid: 9, created: 1 };

    fn focus(game: bool, load_pct: u32, domains: u16) -> FocusProgram {
        FocusProgram {
            key: GAME,
            name: "英雄联盟".into(),
            path: None,
            game,
            load_pct,
            domains,
        }
    }

    fn round(secs: u64, focused: bool, focus: Vec<FocusProgram>, temp: f32) -> Round {
        Round {
            unix_ms: 1_000_000 + secs * 1000,
            focused,
            moved_aside: u32::from(!focus.is_empty()),
            yielded: if focus.is_empty() { vec![] } else { vec![SYNC] },
            focus,
            mode: Mode::Full,
            cpu_pct: 40.0,
            cpu_temp: Some(temp),
            throttled: temp >= 95.0,
        }
    }

    #[test]
    fn a_game_session_is_summarized_when_the_hold_ends() {
        let mut r = SessionRecorder::default();
        // 每 2 秒一轮，玩 60 秒；最后 20 秒处理器到了 96°，被限速。
        for secs in (0..=60).step_by(2) {
            let temp = if secs >= 40 { 96.0 } else { 80.0 };
            let game = vec![focus(true, 300, 0b011)];
            assert!(r.observe(&round(secs, true, game, temp)).is_none());
        }
        // 游戏退出，调度还保持着（两局之间）：不算进时长，也不结束。
        assert!(r.observe(&round(62, true, vec![], 70.0)).is_none());
        let record = r
            .observe(&round(64, false, vec![], 60.0))
            .expect("一段结束");
        assert_eq!(record.ended_unix_ms - record.started_unix_ms, 60_000);
        let [game] = record.programs.as_slice() else {
            panic!("只有游戏是重点程序");
        };
        assert!(game.game);
        assert_eq!((game.peak_load_pct, game.widest_domains), (300, 0b011));
        assert_eq!(game.focused_secs, 60);
        assert_eq!((record.moved_aside_peak, record.yielded_programs), (1, 1));
        assert_eq!(record.modes, vec![Mode::Full]);
        assert_eq!(record.cpu_temp_peak, Some(96.0));
        // 40～60 秒那 11 轮各 2 秒。
        assert_eq!(record.throttled_secs, 22);
    }

    #[test]
    fn sleeping_ends_the_session_and_waking_starts_a_new_one() {
        let mut r = SessionRecorder::default();
        for secs in (0..=40).step_by(2) {
            assert!(
                r.observe(&round(secs, true, vec![focus(true, 200, 1)], 70.0))
                    .is_none()
            );
        }
        // 合上盖子睡了一小时，醒来游戏还开着：之前的一段到此为止。
        let before = r
            .observe(&round(3640, true, vec![focus(true, 200, 1)], 70.0))
            .expect("睡眠前的一段");
        assert_eq!(before.programs[0].focused_secs, 40);
        for secs in (3642..=3680).step_by(2) {
            r.observe(&round(secs, true, vec![focus(true, 200, 1)], 70.0));
        }
        let after = r.finish().expect("醒来后的一段");
        assert_eq!(after.started_unix_ms, 1_000_000 + 3640 * 1000);
        assert_eq!(after.programs[0].focused_secs, 40);
    }

    #[test]
    fn uneven_rounds_add_up_to_whole_seconds() {
        let mut r = SessionRecorder::default();
        let at = |ms: u64| Round {
            unix_ms: 1_000_000 + ms,
            ..round(0, true, vec![focus(true, 200, 1)], 70.0)
        };
        // 每 1.5 秒一轮（立即重新调度插进来的轮次），共 45 秒：不因每轮舍掉零头只剩 30 秒。
        for i in 0..=30 {
            r.observe(&at(i * 1500));
        }
        assert_eq!(r.finish().unwrap().programs[0].focused_secs, 45);
    }

    #[test]
    fn short_bursts_are_not_recorded() {
        let mut r = SessionRecorder::default();
        r.observe(&round(0, true, vec![focus(false, 150, 1)], 70.0));
        r.observe(&round(10, true, vec![focus(false, 150, 1)], 70.0));
        assert!(r.observe(&round(12, false, vec![], 70.0)).is_none());
        // 没有重点程序时不开始。
        assert!(r.observe(&round(20, true, vec![], 70.0)).is_none());
        assert!(r.finish().is_none());
    }

    #[test]
    fn stopping_mid_session_keeps_what_was_recorded() {
        let mut r = SessionRecorder::default();
        for secs in (0..=60).step_by(2) {
            r.observe(&round(secs, true, vec![focus(false, 150, 1)], 70.0));
        }
        let record = r.finish().expect("够长");
        assert_eq!(record.programs[0].widest_domains, 0b001);
        assert!(!record.programs[0].game);
        assert_eq!(record.throttled_secs, 0);
    }
}
