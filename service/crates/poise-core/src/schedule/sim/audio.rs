//! 发声的程序：只保护真正输出声音的进程，短暂停声不来回切换，玩游戏时太忙才挪到小核。

use super::*;

#[test]
fn programs_playing_audio_are_never_touched_even_when_busy() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            cpu_pct: 80.0,
            ..app(2)
        },
        Proc {
            audio: true,
            ..child(3, 2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(10 * 60_000);
    // 发声的程序忙也不让路；只有真正输出声音的进程被保护（一次），其余进程不动。
    let expected: HashMap<Pid, usize> = [(3, 1)].into();
    assert_eq!(sim.adjustments_per_process(), expected);
    assert!(sim.only_protected(3));
}

/// 切歌、暂停、语音里的短暂安静：保持一会儿，不来回切换；停得久了才回到普通程序的规则。
#[test]
fn short_audio_pauses_do_not_flip_the_state() {
    let mut sim = Sim::new(vec![
        app(1),
        Proc {
            cpu_pct: 80.0,
            audio: true,
            ..app(2)
        },
    ]);
    sim.foreground = Some(1);
    sim.run_for(60_000);
    for _ in 0..10 {
        sim.proc_mut(2).audio = false;
        sim.run_for(16_000);
        sim.proc_mut(2).audio = true;
        sim.run_for(30_000);
    }
    assert!(sim.is_protected(2));
    assert_eq!(sim.adjustments_per_process(), HashMap::from([(2, 1)]));
    // 停了很久，又一直在后台占处理器，而用户正在干活：按普通程序的规则让路。
    sim.proc_mut(2).audio = false;
    sim.proc_mut(1).cpu_pct = 80.0;
    sim.run_for(60_000);
    assert!(sim.is_yielded(2));
}

/// 复盘"玩游戏时开着很忙的发声程序（例如直播、带声音的录屏）"：只让到小核，不降优先级、不节能，
/// 声音照样流畅，大核留给游戏；不玩了回到原样（不受系统节能限速）。
#[test]
fn busy_audio_programs_move_to_small_cores_only_while_gaming() {
    const GAME: Pid = 1;
    const STREAMER: Pid = 2;
    let mut sim = Sim::new(vec![
        Proc {
            cpu_pct: 150.0,
            preference: AppPreference::Game,
            ..app(GAME)
        },
        Proc {
            cpu_pct: 60.0,
            audio: true,
            ..app(STREAMER)
        },
    ]);
    sim.foreground = Some(GAME);
    sim.run_for(2 * 60_000);
    assert_eq!(sim.state_of(STREAMER), Some(Desired::BACKSEAT));
    assert!(sim.actions_on(STREAMER).iter().all(|a| !matches!(
        a,
        Action::SetPriority { .. }
            | Action::SetEcoQos {
                state: EcoQos::On,
                ..
            }
    )));
    // 不玩了。
    sim.procs.retain(|p| p.pid != GAME);
    sim.foreground = Some(STREAMER);
    sim.run_for(TICK_MS);
    assert_eq!(sim.state_of(STREAMER), Some(Desired::PROTECTED));
}
