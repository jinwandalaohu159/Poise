using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Monitoring;
using Poise.Core.Presentation;
using Poise.Core.Settings;

namespace Poise.Core.Tests;

/// <summary>
/// 把服务数据换成界面文案与指标：HomeViewBuilder、CleanupText、CleanupPageText、ErrorText、HardwareText、
/// DiagnoseText、OptimizeSummary。
/// </summary>
public sealed class PresentationTests
{
    private static readonly DateTimeOffset Now = DateTimeOffset.Now;

    public PresentationTests() => TestSupport.UseChinese();

    private static Status Full => TestSupport.Fixture<Status>("status_full.json");

    private static Status Minimal => TestSupport.Fixture<Status>("status_minimal.json");

    private static Status WithHealth(Status s, HealthCode code, Level level = Level.Notice) =>
        s with { Health = s.Health with { Code = code, Level = level } };

    // ---- 主页 ----

    [Fact]
    public void Disconnected_home_has_no_values()
    {
        var v = HomeViewBuilder.Build(null, Now);

        Assert.False(v.Connected);
        Assert.Null(v.CpuLoad.Value);
        Assert.Null(v.CpuTemp.Fraction);
        Assert.Null(v.Memory.Value);
        Assert.False(v.CanSnoozeRestart);
    }

    [Fact]
    public void Fan_line_shows_one_speed_against_the_maximum()
    {
        TestSupport.UseChinese();

        var fan = HomeViewBuilder.Build(Full, Now).Fan!;

        Assert.Equal((60, "当前 2,500 / 4,800 RPM"), (fan.Boost, fan.Speed));
        Assert.Equal(("自动", "60%"), (FanView.BoostText(0), FanView.BoostText(60)));
        Assert.Null(HomeViewBuilder.Build(Minimal, Now).Fan);
    }

    [Fact]
    public void Modes_include_the_machine_mode_only_when_supported()
    {
        TestSupport.UseChinese();

        var withG = ModeText.Options("dell_g_mode", Mode.Auto, Mode.Balanced);
        Assert.Equal([Mode.Auto, Mode.Quiet, Mode.Balanced, Mode.Full, Mode.Vendor], withG.Select(o => o.Mode));
        Assert.Equal(("G 模式", "当前：均衡"), (withG[4].Title, withG[0].Description));
        Assert.Equal(4, ModeText.Options(null, Mode.Full, Mode.Full).Count);
        Assert.Equal(Loc.Get("mode.auto.desc"), ModeText.Options(null, Mode.Full, Mode.Full)[0].Description);

        Assert.Equal([GameMode.Off, GameMode.Full, GameMode.Vendor], ModeText.GameOptions("dell_g_mode").Select(o => o.Mode));
        Assert.Equal([GameMode.Off, GameMode.Full], ModeText.GameOptions(null).Select(o => o.Mode));
        // 机型不支持极限模式：下拉框显示全力。
        Assert.Equal(GameMode.Full, ModeText.ShownGameOption(ModeText.GameOptions(null), GameMode.Vendor).Mode);
        Assert.Equal(GameMode.Off, ModeText.ShownGameOption(ModeText.GameOptions(null), GameMode.Off).Mode);
    }

    [Fact]
    public void Only_restart_advice_can_be_snoozed()
    {
        var restart = HomeViewBuilder.Build(WithHealth(Full, HealthCode.RestartRecommended), Now);
        Assert.True(restart.CanSnoozeRestart);
        Assert.Equal((Level.Notice, true), (restart.Notice!.Level, restart.Notice.CanSnooze));
        Assert.False(HomeViewBuilder.Build(WithHealth(Full, HealthCode.Tired), Now).CanSnoozeRestart);
    }

    [Fact]
    public void Observe_only_mode_is_a_plain_notice_and_otherwise_there_is_none()
    {
        // 样本里是观察模式。
        var running = Full with { Paused = false };
        var observe = HomeViewBuilder.Build(WithHealth(running, HealthCode.Tired), Now).Notice!;
        Assert.Equal((Level.Good, false), (observe.Level, observe.CanSnooze));
        Assert.Null(HomeViewBuilder.Build(WithHealth(running with { DryRun = false }, HealthCode.Tired), Now).Notice);
    }

    [Fact]
    public void Paused_is_a_warning_after_restart()
    {
        // 样本里暂停着：比观察模式要紧，比建议重启次要。
        var paused = HomeViewBuilder.Build(WithHealth(Full, HealthCode.Tired), Now).Notice!;
        Assert.Equal(("已暂停", Level.Notice, false), (paused.Title, paused.Level, paused.CanSnooze));
        Assert.True(HomeViewBuilder.Build(WithHealth(Full, HealthCode.RestartRecommended), Now).Notice!.CanSnooze);
    }

    [Fact]
    public void Four_metrics_with_units_and_bar_lengths()
    {
        var v = HomeViewBuilder.Build(Full, Now);

        Assert.Equal((12.5, "%"), (v.CpuLoad.Value!.Value, v.CpuLoad.Unit));
        Assert.Equal(0.125, v.CpuLoad.Fraction!.Value, 3);
        Assert.Equal((71.0, "°C"), (v.CpuTemp.Value!.Value, v.CpuTemp.Unit));
        Assert.Equal(0.71, v.CpuTemp.Fraction!.Value, 3);
        Assert.Equal(58.0, v.GpuTemp.Value);
        Assert.Equal(0.63, v.Memory.Fraction!.Value, 3);
    }

    [Fact]
    public void Missing_temperatures_show_no_value()
    {
        var v = HomeViewBuilder.Build(Minimal, Now);

        Assert.Null(v.CpuTemp.Value);
        Assert.Null(v.GpuTemp.Value);
        Assert.Equal(63.0, v.Memory.Value!.Value, 1);
    }

    [Theory]
    [InlineData(70f, Level.Good)]
    [InlineData(85f, Level.Notice)]
    [InlineData(95f, Level.Bad)]
    public void Temperature_level_follows_thresholds(float celsius, Level expected)
    {
        var s = Full with { Metrics = Full.Metrics with { CpuTemp = celsius } };

        Assert.Equal(expected, HomeViewBuilder.Build(s, Now).CpuTemp.Level);
    }

    [Fact]
    public void Cleanup_card_shows_items_last_cleanup_and_freeable_share()
    {
        var withLast = HomeViewBuilder.Build(Full, Now).Cleanup;
        var nothing = HomeViewBuilder.Build(Minimal, Now).Cleanup;

        Assert.Equal((1UL << 30, 2), (withLast.FreeableBytes, withLast.Items));
        Assert.StartsWith(Loc.Format("home.cleanup.items", 2), withLast.Detail);
        Assert.Contains("2.0 GB", withLast.Detail);
        Assert.Equal(0.63, withLast.MemoryFraction, 3);
        Assert.Equal(1.0 / 32, withLast.FreeableFraction, 3);
        Assert.Equal(Loc.Get("home.cleanup.none"), nothing.Detail);
    }

    // ---- 清理结果 ----

    [Fact]
    public void Cleanup_result_reports_freed_space_and_details()
    {
        var r = TestSupport.Fixture<CleanupResult>("cleanup_result.json");

        var f = CleanupText.Describe(r, dryRun: false);

        Assert.Equal(FeedbackKind.Success, f.Kind);
        Assert.Equal(Loc.Format("clean.done", "1.1 GB"), f.Message);
        Assert.Contains(Loc.Format("clean.deferred", 1), f.Details);
        Assert.Contains(Loc.Format("clean.heavy_app", "微信"), f.Details);
        Assert.Contains(Loc.Get("clean.restart"), f.Details);
        Assert.Contains(Loc.Format("clean.failed", 1), f.Details);
    }

    [Fact]
    public void Observe_mode_lists_what_would_have_been_done()
    {
        var r = TestSupport.Fixture<CleanupResult>("cleanup_result.json");

        var f = CleanupText.Describe(r, dryRun: true);

        Assert.Equal(FeedbackKind.Info, f.Kind);
        Assert.Equal(Loc.Get("clean.observe"), f.Message);
        Assert.Equal(r.Handled[0], f.Details[0]);
    }

    [Fact]
    public void Nothing_to_clean_is_informational()
    {
        var f = CleanupText.Describe(new CleanupResult(0, [], [], 0, []), dryRun: false);

        Assert.Equal(FeedbackKind.Info, f.Kind);
        Assert.Equal(Loc.Get("clean.nothing"), f.Message);
    }

    // ---- 错误 ----

    [Theory]
    [InlineData(ErrorCode.ObserveOnly, "error.observe_only")]
    [InlineData(ErrorCode.ProcessGone, "error.process_gone")]
    [InlineData(ErrorCode.InvalidPath, "error.invalid_path")]
    [InlineData(ErrorCode.NotReady, "error.not_ready")]
    [InlineData(ErrorCode.NotFound, "error.not_found")]
    [InlineData(ErrorCode.Failed, "error.failed")]
    [InlineData(ErrorCode.Internal, "error.generic")]
    [InlineData(ErrorCode.Unknown, "error.generic")]
    public void Service_errors_map_to_text_keys(ErrorCode code, string key) =>
        Assert.Equal(key, ErrorText.Key(new ServiceErrorException(code, "detail")));

    [Fact]
    public void Unavailable_service_has_its_own_text()
    {
        Assert.Equal("error.unavailable", ErrorText.Key(new ServiceUnavailableException("x")));
        Assert.Equal("error.generic", ErrorText.Key(new InvalidOperationException()));
    }

    // ---- 硬件信息 ----

    [Fact]
    public void In_use_badge_only_marks_a_game_or_overheat_switch()
    {
        // 所选安静、实际均衡：没有游戏也没有过热，是切换的中间状态，不标。
        var transient = Full with { Mode = Mode.Quiet, EffectiveMode = Mode.Balanced, Health = Full.Health with { Code = HealthCode.Smooth, Level = Level.Good, Game = null } };
        Assert.Null(HomeViewBuilder.Build(transient, Now).InUse);

        // 玩游戏时切到全力：标在全力上。
        var game = transient with { EffectiveMode = Mode.Full, Health = transient.Health with { Code = HealthCode.InGame, Game = "League of Legends" } };
        Assert.Equal(Mode.Full, HomeViewBuilder.Build(game, Now).InUse);

        // 过热降档：标在均衡上；选"自动"时不标。
        var hot = transient with { Mode = Mode.Full, Health = transient.Health with { Code = HealthCode.Hot, Level = Level.Bad } };
        Assert.Equal(Mode.Balanced, HomeViewBuilder.Build(hot, Now).InUse);
        Assert.Null(HomeViewBuilder.Build(hot with { Mode = Mode.Auto }, Now).InUse);
    }

    [Fact]
    public void Hardware_rows_include_everything_that_was_read()
    {
        var rows = HardwareText.Rows(Full).ToDictionary(r => r.Label, r => r.Value);

        Assert.Equal("Dell G15 5520", rows[Loc.Get("settings.device")]);
        Assert.Equal("6 个性能核（12 线程）+ 8 个能效核（8 线程）", rows[Loc.Get("hw.cpu")]);
        Assert.Equal("71°C", rows[Loc.Get("hw.cpu_temp")]);
        Assert.Equal(Loc.Format("hw.fan_value", "2,500", "4,800"), rows[Loc.Get("hw.fans")]);
        Assert.Equal("32 GB · 16 GB × 2 · 4800 MHz", rows[Loc.Get("hw.memory")]);
        Assert.Equal($"WD SN750 2TB · {Loc.Format("hw.disk_usage", "488 GB", "2 TB")} · {Loc.Get("hw.disk.warning")} · 45°C · {Loc.Format("hw.disk_wear", 3)}",
            rows[Loc.Get("hw.disk")]);
        Assert.Equal(Loc.Format("hw.system_drive_value", "12 GB", "500 GB"), rows[Loc.Format("hw.volume_system", "C")]);
        Assert.Equal(Loc.Get("settings.thermal.on"), rows[Loc.Get("settings.thermal")]);
    }

    [Fact]
    public void Hardware_rows_skip_what_could_not_be_read()
    {
        var labels = HardwareText.Rows(Minimal).Select(r => r.Label).ToList();

        Assert.Equal([Loc.Get("settings.device"), Loc.Get("hw.disk"), Loc.Get("settings.thermal")], labels);
    }

    // ---- 调度 ----

    [Fact]
    public void Schedule_state_reads_like_a_status_line()
    {
        Assert.Equal("后台 · 能效核 · 优先级较低",
            ScheduleText.State(new AppSchedule(ScheduleState.Scheduled, false, CoreUse.Efficiency, PriorityLevel.Lower, Adjustment.Yielded)));
        // 不是大小核处理器：不提核心。
        Assert.Equal("后台 · 优先级较低",
            ScheduleText.State(new AppSchedule(ScheduleState.Scheduled, false, null, PriorityLevel.Lower, Adjustment.Yielded)));
        // 没调整过：程序自己的优先级不当成 Poise 的调整。
        Assert.Equal("前台 · 未调整",
            ScheduleText.State(new AppSchedule(ScheduleState.Scheduled, true, CoreUse.Any, PriorityLevel.Higher, Adjustment.None)));
        Assert.Equal("后台 · 不受系统节能限速",
            ScheduleText.State(new AppSchedule(ScheduleState.Scheduled, false, CoreUse.Any, PriorityLevel.Normal, Adjustment.Protected)));
        // 正在玩的游戏：固定在大核。
        Assert.Equal("后台 · 性能核 · 不受系统节能限速",
            ScheduleText.State(new AppSchedule(ScheduleState.Scheduled, false, CoreUse.Performance, PriorityLevel.Normal, Adjustment.Protected)));
        // 重点程序（前台持续高负载）：只安排了核心。
        Assert.Equal("前台 · 性能核 · 优先级正常",
            ScheduleText.State(new AppSchedule(ScheduleState.Scheduled, true, CoreUse.Performance, PriorityLevel.Normal, Adjustment.Placed)));
        Assert.Equal("前台 · 性能核 · 优先级较高 · 不受系统节能限速",
            ScheduleText.State(new AppSchedule(ScheduleState.Scheduled, true, CoreUse.Performance, PriorityLevel.Higher, Adjustment.Raised)));
        Assert.Equal("未调整（由程序自行管理）",
            ScheduleText.State(new AppSchedule(ScheduleState.SelfManaged, false, CoreUse.Efficiency, PriorityLevel.Normal, Adjustment.None)));
        Assert.Null(ScheduleText.State(null));
    }

    [Fact]
    public void Processor_without_small_cores_shows_one_count()
    {
        Assert.Equal("8 核（16 线程）", HardwareText.Processor(new CpuInfo(8, 16, 0, 0)));
    }

    [Fact]
    public void Diagnose_shows_the_smart_schedule_switch()
    {
        var d = TestSupport.Fixture<Diagnosis>("diagnosis.json");

        Assert.Equal("智能调度：已开启", DiagnoseText.Build(d).SmartSchedule);
    }

    [Fact]
    public void Trend_loads_show_both_core_kinds_or_whole_cpu()
    {
        var t = TrendSeries.From(TestSupport.Fixture<Trend>("trend.json"), TimeSpan.FromSeconds(100));

        Assert.Equal("性能核 64% · 能效核 18%", TrendText.Loads(t.Samples[0]));
        Assert.Equal("CPU 占用 92%", TrendText.Loads(t.Samples[1]));
        Assert.Equal("", TrendText.Loads(null));
    }

    [Fact]
    public void Memory_lists_mixed_modules_one_by_one()
    {
        const ulong gb = 1UL << 30;
        var mixed = new MemoryInfo(24 * gb, 0, [8 * gb, 16 * gb]);

        Assert.Equal("24 GB · 16 GB + 8 GB", HardwareText.Memory(mixed));
    }

    // ---- 清理页 ----

    [Fact]
    public void Cleanup_page_splits_rows_and_explains_each_one()
    {
        var v = CleanupPageText.Build(TestSupport.Fixture<CleanupOverview>("cleanup_overview.json"), Now);

        Assert.Equal(3, v.NeedsAttention);
        Assert.Equal([1u, 20u, 40u], v.Attention.Select(r => r.Pid));
        var audio = v.Attention[0];
        Assert.Equal((Loc.Get("issue.oversized"), TagTone.Notice), (audio.Status, audio.Tone));
        Assert.Equal(Loc.Get("remedy.restart_audio"), audio.Action);
        Assert.Equal("", audio.ProcessCount);
        var wechat = v.Attention[1];
        Assert.Equal(Loc.Format("issue.growing_rate", "80 MB"), wechat.Status);
        Assert.Equal(Loc.Format("cleanup.processes", 12), wechat.ProcessCount);
        Assert.Equal(Loc.Get("remedy.reopen"), wechat.Action);
        var edge = Assert.Single(v.Others);
        Assert.Equal((Loc.Get("cleanup.normal"), TagTone.None, ""), (edge.Status, edge.Tone, edge.Action));
        Assert.Equal(1.0, edge.MemoryFraction, 3);
        Assert.True(v.RestartAdvised);
        Assert.Equal(20.0 / 32, v.MemoryUsedFraction, 3);
    }

    [Fact]
    public void Processes_grown_since_start_say_by_how_much_and_why_they_are_left_alone()
    {
        TestSupport.UseChinese();
        var v = CleanupPageText.Build(TestSupport.Fixture<CleanupOverview>("cleanup_overview.json"), Now);

        var push = v.Attention.Single(r => r.Pid == 40);
        Assert.Equal(("句柄比启动时多 4.4 万", TagTone.Notice), (push.Status, push.Tone));
        Assert.Equal(Loc.Get("remedy.none"), push.Action);
    }

    [Fact]
    public void Since_boot_card_compares_each_total_and_marks_the_notable_ones()
    {
        TestSupport.UseChinese();
        var v = CleanupPageText.Build(TestSupport.Fixture<CleanupOverview>("cleanup_overview.json"), Now).SinceBoot;

        var recorded = Format.When(DateTimeOffset.FromUnixTimeMilliseconds(1_793_000_000_000), Now);
        Assert.Equal($"已开机 3 天 0 小时 · 基线记录于 {recorded}", v.Caption);
        Assert.Equal(1, v.NotableCount);
        Assert.Equal(new SinceBootRow("句柄", "8.2 万", "16.8 万", "+8.6 万", true), v.Rows[0]);
        Assert.Equal(new SinceBootRow("内核内存", "700 MB", "900 MB", "+200 MB", false), v.Rows[1]);
        Assert.Equal(new SinceBootRow("系统缓存", "1.0 GB", "6.0 GB", "+5.0 GB", false), v.Rows[2]);
    }

    [Fact]
    public void Since_boot_card_explains_when_the_baseline_will_exist()
    {
        var v = SinceBootText.Build(null, Now);

        Assert.Equal((Loc.Get("since_boot.pending"), 0), (v.Caption, v.Rows.Count));
    }

    [Theory]
    [InlineData("restart_service", "Spooler", "remedy.restart_service")]
    [InlineData("recreate_compositor", null, "remedy.recreate_compositor")]
    [InlineData("restart_app", null, "remedy.restart_app")]
    [InlineData("none", null, "")]
    public void Remedies_have_their_own_text(string kind, string? service, string key)
    {
        var text = CleanupPageText.Action(new Remedy(kind, service));

        var expected = key.Length == 0 ? "" : service is null ? Loc.Get(key) : Loc.Format(key, service);
        Assert.Equal(expected, text);
    }

    // ---- 系统优化进度 ----

    [Fact]
    public void Optimize_summary_counts_recommended_items_only()
    {
        static ItemStatus Item(string id, bool oneClick, string state) => new(
            new OptimizationItem(id, "explorer", new TextValue("plain", id), new TextValue("plain", ""), oneClick, false, "immediately"),
            state, false);
        var s = OptimizeSummary.From([
            Item("a", true, "optimized"), Item("b", true, "not_optimized"), Item("plugin", false, "not_optimized"),
        ]);

        Assert.Equal((1, 2), (s.Optimized, s.Total));
        Assert.Equal(0.5, s.Fraction, 3);
        Assert.Equal(Loc.Format("home.optimize.remaining", 1), s.Detail);
    }

    // ---- 卡顿诊断 ----

    [Fact]
    public void Diagnosis_gives_evidence_actions_and_rankings()
    {
        var v = DiagnoseText.Build(TestSupport.Fixture<Diagnosis>("diagnosis.json"));

        Assert.Equal(Loc.Format("diagnose.background_cpu", "OneDrive"), v.Conclusion);
        Assert.Equal(Loc.Format("diagnose.evidence.share", "92", "OneDrive", "35"), v.Evidence);
        Assert.Equal(Level.Notice, v.Level);
        Assert.Equal(SuggestionKind.SetLowPriority, v.Suggestions[0].Kind);
        Assert.EndsWith("onedrive.exe", v.Suggestions[0].Path);
        Assert.Equal(SuggestionKind.OpenApps, v.Suggestions[1].Kind);
        Assert.Equal(["OneDrive", "Chrome"], v.TopCpu.Select(b => b.Name));
        Assert.Equal(["35.0%", "4.5%"], v.TopCpu.Select(b => b.Value));
        Assert.Equal(1.0, v.TopCpu[0].Fraction, 3);
        Assert.Equal(4.5 / 35, v.TopCpu[1].Fraction, 3);
        Assert.Equal("3.0 GB", Assert.Single(v.TopMemory).Value);
    }

    [Theory]
    [InlineData("{\"kind\":\"thermal\"}", SuggestionKind.OpenOptimize)]
    [InlineData("{\"kind\":\"memory_pressure\"}", SuggestionKind.OpenCleanup)]
    [InlineData("{\"kind\":\"leaks\",\"count\":3}", SuggestionKind.OpenCleanup)]
    [InlineData("{\"kind\":\"foreground_heavy\",\"name\":\"Game\",\"share_pct\":60.0}", SuggestionKind.SwitchToFull)]
    [InlineData("{\"kind\":\"unclear\"}", SuggestionKind.OpenOptimize)]
    public void Every_cause_suggests_a_next_step(string cause, SuggestionKind expected)
    {
        var d = new Diagnosis(System.Text.Json.JsonDocument.Parse(cause).RootElement, 50, 60, 95, true, [], [], true);

        var v = DiagnoseText.Build(d);

        Assert.Equal(expected, v.Suggestions[0].Kind);
        Assert.False(string.IsNullOrWhiteSpace(v.Evidence));
        Assert.DoesNotContain("{", v.Conclusion + v.Evidence);
    }

    [Fact]
    public void App_detail_adds_the_process_count_for_multi_process_apps()
    {
        var apps = TestSupport.Fixture<List<AppEntry>>("apps.json");

        Assert.Equal("前台 · 性能核 · 优先级较高 · 不受系统节能限速 · 2 个进程", AppsText.Detail(apps[1]));
        Assert.Equal("后台 · 能效核 · 优先级较低", AppsText.Detail(apps[0]));
        Assert.Equal(apps[^1].Path, AppsText.Detail(apps[^1]));
        Assert.Equal(("6.5%", "3.0 GB"), (AppsText.Cpu(apps[1].CpuPct), AppsText.Memory(apps[1].MemoryBytes)));
    }

    [Fact]
    public void Apps_sort_by_any_column_in_either_direction_with_name_as_tie_breaker()
    {
        var rows = new[] { ("b", 1f, 300UL), ("a", 5f, 100UL), ("c", 5f, 200UL) };

        string Names(AppSort column, bool descending) =>
            string.Concat(AppsText.Order(rows, new AppSortState(column, descending), r => r.Item1, r => r.Item2, r => r.Item3).Select(r => r.Item1));

        Assert.Equal("abc", Names(AppSort.Name, false));
        Assert.Equal("cba", Names(AppSort.Name, true));
        Assert.Equal("acb", Names(AppSort.Cpu, true));
        Assert.Equal("bac", Names(AppSort.Cpu, false));
        Assert.Equal("bca", Names(AppSort.Memory, true));
        Assert.Equal("acb", Names(AppSort.Memory, false));
    }

    [Fact]
    public void Rows_with_similar_usage_keep_their_places_until_clearly_out_of_order()
    {
        var shown = new[] { ("a", 10.0), ("b", 9.0), ("c", 1.0) };
        string Settle((string, double)[] fresh, double slack = 0.5) =>
            string.Concat(AppsText.Settle(shown, fresh.OrderByDescending(r => r.Item2).ToList(), r => r.Item1, r => r.Item2, true, slack).Select(r => r.Item1));

        // b 稍微超过 a：不换位。
        Assert.Equal("abc", Settle([("a", 10.0), ("b", 10.5), ("c", 1.0)]));
        // c 明显超过了 b：按新顺序。
        Assert.Equal("cab", Settle([("a", 10.0), ("b", 9.0), ("c", 30.0)]));
        // 差得多但绝对值很小（0.1% 对 0.3%）：不换位。
        Assert.Equal("abc", Settle([("a", 0.1), ("b", 0.0), ("c", 0.3)]));
        // 行变了：按新顺序。
        Assert.Equal("ba", Settle([("a", 1.0), ("b", 2.0)]));
    }

    [Fact]
    public void Clicking_a_column_uses_its_natural_direction_and_clicking_again_reverses()
    {
        var byName = AppSortState.Default;

        Assert.Equal(new AppSortState(AppSort.Name, true), byName.Click(AppSort.Name));
        Assert.Equal(new AppSortState(AppSort.Cpu, true), byName.Click(AppSort.Cpu));
        Assert.Equal(new AppSortState(AppSort.Cpu, false), byName.Click(AppSort.Cpu).Click(AppSort.Cpu));
        Assert.Equal(new AppSortState(AppSort.Memory, true), byName.Click(AppSort.Cpu).Click(AppSort.Cpu).Click(AppSort.Memory));
        Assert.Equal(byName, byName.Click(AppSort.Cpu).Click(AppSort.Name));
        Assert.Equal("按 CPU 从高到低排序", AppsText.SortHint(byName, AppSort.Cpu));
        Assert.Equal("按名称排序（Z→A）", AppsText.SortHint(byName, AppSort.Name));
    }

    [Fact]
    public void App_names_interleave_chinese_by_pinyin_initial_with_english()
    {
        string[] names = ["微信", "百度网盘", "docker", "Chrome", "钉钉", "7-Zip", "爱奇艺", "Zoom", "腾讯会议"];

        Assert.Equal(["7-Zip", "爱奇艺", "百度网盘", "Chrome", "钉钉", "docker", "腾讯会议", "微信", "Zoom"], names.Order(NameOrder.Instance));
    }

    [Theory]
    [InlineData("阿里旺旺", 'A')]
    [InlineData("百度网盘", 'B')]
    [InlineData("钉钉", 'D')]
    [InlineData("飞书", 'F')]
    [InlineData("酷狗音乐", 'K')]
    [InlineData("网易云音乐", 'W')]
    [InlineData("迅雷", 'X')]
    [InlineData("有道词典", 'Y')]
    [InlineData("知乎", 'Z')]
    [InlineData("qq", 'Q')]
    [InlineData(".NET Host", '#')]
    public void Name_initials_follow_pinyin(string name, char initial)
    {
        Assert.Equal(initial, NameOrder.Initial(name));
    }

    [Fact]
    public void Fan_speed_never_shows_above_the_maximum()
    {
        Assert.Equal("4,800 / 4,800 RPM", FanText.Speed([new FanInfo(4819, 4800), new FanInfo(4811, 4800)]));
        Assert.Equal("3,900 / 4,800 RPM", FanText.Speed([new FanInfo(3900, 4800)]));
        Assert.Equal("2,000 / 0 RPM", FanText.Speed([new FanInfo(2000, 0)]));
    }
}
