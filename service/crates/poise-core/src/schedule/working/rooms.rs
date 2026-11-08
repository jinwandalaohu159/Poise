//! 在干活的程序能用多少处理器（见 `policy::WorkRoom`），规则对所有程序都一样：
//!
//! - 用户正在用它，或者没人和它抢：全用；
//! - 用户此刻直接在用的别的程序在忙：大小核照用，优先级让一步（刚用过的、在后台干活的不算——
//!   工作和工作之间不分先后）；
//! - 有重点程序：大核按物理核心切开，重点程序实际要几个就留几个（看结果调整，见 `trackers::share`），
//!   它用其余核心；副域一样快（多 CCD）时整个主域留给重点程序，它用别的域；重点程序要整个大核时只用小核；
//! - 有重点程序时处理器发热、或正被功耗或温度压着：余量都给重点程序，它只用小核。

use super::super::policy::WorkRoom;
use super::super::program::Program;
use super::super::{ScheduleInput, Scheduler};
use crate::action::CoreSet;
use crate::fast_hash::HashMap;
use crate::model::{Millis, Pid, ProcessKey};

/// 发热、被功耗或温度压着之后，干活的程序在小核上至少再待这么久：移开后频率回升，马上搬回来又会被压下去。
const SQUEEZE_HOLD_MS: Millis = 2 * 60_000;

/// 这一轮在干活的程序各能用多少处理器。
pub(in super::super) struct WorkRooms {
    /// 有重点程序时留给它的大核数（物理核心）；没有重点程序时为 None。
    kept: Option<u32>,
    /// 主域的物理核心数。
    main_cores: u32,
    /// 副域和主域一样快（多 CCD）：重点程序留着整个主域，干活的程序用别的域。
    away_equal: bool,
    /// 余量都给重点程序（见 [`Scheduler::squeezed`]）。
    squeezed: bool,
    /// 这一轮的重点程序（和自己分享不算）。
    focused: Vec<ProcessKey>,
    /// 用户此刻直接在用、又在忙的程序（和自己抢不算）。
    busy_in_use: Vec<ProcessKey>,
}

impl WorkRooms {
    /// `in_use`：用户正在用它（前台、刚用过，见 `policy::in_use`，发声不算）——它就是被等着的那个，全用。
    /// 在发声的照样按竞争分（只让核心不让优先级）；玩游戏时刚用过不算正在用，不该占着游戏的大核。
    pub(in super::super) fn of(&self, root: ProcessKey, in_use: bool) -> WorkRoom {
        if in_use {
            return WorkRoom::Free;
        }
        let others = |list: &[ProcessKey]| list.iter().any(|r| *r != root);
        match self.kept {
            Some(_) if self.squeezed => WorkRoom::Aside,
            Some(kept) if others(&self.focused) && kept >= self.main_cores => WorkRoom::Aside,
            Some(_) if others(&self.focused) && self.away_equal => {
                WorkRoom::Shared(CoreSet::Efficiency)
            }
            Some(kept) if others(&self.focused) => {
                WorkRoom::Shared(CoreSet::Rest(kept.min(u32::from(u8::MAX)) as u8))
            }
            _ if others(&self.busy_in_use) => WorkRoom::Contended,
            _ => WorkRoom::Free,
        }
    }
}

impl Scheduler {
    /// 这一轮在干活的程序各能用多少处理器（在判断完谁是重点程序之后）。别的程序是重点程序、
    /// 在用又在忙时才让：一个程序自己从前台的重点程序变成在后台干活时，不和自己抢。
    pub(in super::super) fn work_rooms(
        &mut self,
        input: &ScheduleInput,
        programs: &HashMap<Pid, Program>,
    ) -> WorkRooms {
        let now = input.snapshot.at;
        let in_game = self.games.in_session(now, self.cfg.game_session_hold_ms);
        let leading: Vec<&Program> = programs
            .values()
            .filter(|p| p.focus.is_some() || p.active_game || p.pending_game)
            .collect();
        let main_cores = self.cfg.cores.cores(0);
        let kept = if leading.is_empty() && !in_game {
            self.work.share.reset();
            None
        } else {
            Some(self.kept_for(&leading, main_cores, input))
        };
        let busy_in_use = programs
            .values()
            .filter(|p| {
                self.load.sustained(&p.root) >= self.cfg.demand.in_use_core_pct
                    && input
                        .classes
                        .get(&p.class_pid)
                        .is_some_and(|c| c.in_use_by_user() || p.audible)
            })
            .map(|p| p.root)
            .collect();
        WorkRooms {
            kept,
            main_cores,
            away_equal: self.cfg.cores.away_equal(),
            squeezed: self.squeezed(input, kept.is_some()),
            focused: leading.iter().map(|p| p.root).collect(),
            busy_in_use,
        }
    }

    /// 留给重点程序的大核数（物理核心）：按它们实际用了多少、留出的核心挤不挤（见 `trackers::share`）。
    /// 重点程序已经用着副域（需要的线程超过了主域）时，整个主域都给它们。
    fn kept_for(&mut self, leading: &[&Program], main_cores: u32, input: &ScheduleInput) -> u32 {
        let domains = usize::from(self.cfg.cores.domains());
        let beyond_main = leading
            .iter()
            .any(|p| p.focus.is_some_and(|c| c.domain_mask(domains) & !1 != 0));
        let load: f32 = leading
            .iter()
            .map(|p| {
                self.load
                    .of(&p.root)
                    .core_pct
                    .max(self.load.recent(&p.root))
            })
            .sum();
        let kept = self.work.share.update(
            load,
            main_cores,
            input.main_core_busy,
            self.cfg.share,
            input.snapshot.at,
        );
        if beyond_main { main_cores } else { kept }
    }

    /// 有重点程序时处理器发热、或大核正被功耗或温度压着（和同样多的大核在忙时跑到过的频率比，见
    /// `diagnose::ClockWatch`）：功耗和温度的余量都给重点程序，干活的程序只用小核，至少保持
    /// [`SQUEEZE_HOLD_MS`]。没有重点程序时不这样做：那时干活的程序本身就是用户要的。
    fn squeezed(&mut self, input: &ScheduleInput, leading: bool) -> bool {
        let now = input.snapshot.at;
        if !leading {
            self.work.squeezed_until = 0;
            return false;
        }
        if input.hot || input.cpu_held {
            self.work.squeezed_until = now + SQUEEZE_HOLD_MS;
        }
        now < self.work.squeezed_until
    }
}
