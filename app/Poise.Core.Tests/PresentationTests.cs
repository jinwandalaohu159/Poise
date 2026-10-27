using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Presentation;

namespace Poise.Core.Tests;

/// <summary>把服务状态换成界面文案与指标（HomeViewBuilder、CleanupText、ErrorText、HardwareText、DiagnoseText）。</summary>
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
        Assert.Null(v.Level);
        Assert.Equal(Loc.Get("status.disconnected"), v.Title);
        Assert.Null(v.Cpu.Value);
        Assert.Null(v.Memory.Value);
        Assert.False(v.CanSnoozeRestart);
    }

    [Theory]
    [InlineData(HealthCode.Smooth, "status.smooth")]
    [InlineData(HealthCode.Tired, "status.tired")]
    [InlineData(HealthCode.Hot, "status.hot")]
    [InlineData(HealthCode.RestartRecommended, "status.restart")]
    [InlineData(HealthCode.StorageLow, "status.storage")]
    [InlineData(HealthCode.InGame, "status.in_game")]
    public void Each_health_code_has_its_own_title_and_description(HealthCode code, string key)
    {
        var v = HomeViewBuilder.Build(WithHealth(Full, code), Now);

        Assert.Equal(Loc.Get(key), v.Title);
        Assert.False(string.IsNullOrWhiteSpace(v.Description));
        Assert.DoesNotContain("{0}", v.Description);
    }

    [Fact]
    public void In_game_names_the_game_and_whether_full_power_is_on()
    {
        var full = HomeViewBuilder.Build(WithHealth(Full, HealthCode.InGame), Now);
        var notFull = HomeViewBuilder.Build(WithHealth(Full, HealthCode.InGame) with { EffectiveMode = Mode.Balanced }, Now);

        Assert.Equal(Loc.Format("status.in_game.desc", "League of Legends"), full.Description);
        Assert.Equal(Loc.Format("status.in_game.desc_no_full", "League of Legends"), notFull.Description);
    }

    [Fact]
    public void Only_restart_advice_can_be_snoozed()
    {
        Assert.True(HomeViewBuilder.Build(WithHealth(Full, HealthCode.RestartRecommended), Now).CanSnoozeRestart);
        Assert.False(HomeViewBuilder.Build(WithHealth(Full, HealthCode.Tired), Now).CanSnoozeRestart);
    }

    [Fact]
    public void Cpu_shows_temperature_when_available_otherwise_load()
    {
        Assert.Equal(Loc.Get("metric.cpu_temp"), HomeViewBuilder.Build(Full, Now).Cpu.Label);
        Assert.Equal(Loc.Get("metric.cpu_load"), HomeViewBuilder.Build(Minimal, Now).Cpu.Label);
    }

    [Theory]
    [InlineData(70f, Level.Good)]
    [InlineData(85f, Level.Notice)]
    [InlineData(95f, Level.Bad)]
    public void Temperature_level_follows_thresholds(float celsius, Level expected)
    {
        var s = Full with { Metrics = Full.Metrics with { CpuTemp = celsius } };

        Assert.Equal(expected, HomeViewBuilder.Build(s, Now).Cpu.Level);
    }

    [Fact]
    public void Auto_clean_line_shows_the_last_cleanup()
    {
        var withLast = HomeViewBuilder.Build(Full, Now);
        var without = HomeViewBuilder.Build(Minimal, Now);

        Assert.Contains("2.0 GB", withLast.AutoCleanDetail);
        Assert.Equal(Loc.Get("home.auto_clean.desc"), without.AutoCleanDetail);
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
    public void Hardware_rows_include_everything_that_was_read()
    {
        var rows = HardwareText.Rows(Full).ToDictionary(r => r.Label, r => r.Value);

        Assert.Equal("Dell G15 5520", rows[Loc.Get("settings.device")]);
        Assert.Equal("71°C", rows[Loc.Get("hw.cpu_temp")]);
        Assert.Equal("2400 RPM · 2600 RPM", rows[Loc.Get("hw.fans")]);
        Assert.Equal(Loc.Format("hw.memory_value", "32.0 GB", 4800, 2), rows[Loc.Get("hw.memory")]);
        Assert.Equal($"45°C · {Loc.Get("hw.disk.warning")} · {Loc.Format("hw.disk_wear", 3)}", rows["WD SN750 2TB"]);
        Assert.Equal(Loc.Format("hw.system_drive_value", "12.0 GB", "500.0 GB"), rows[Loc.Get("hw.system_drive")]);
        Assert.Equal(Loc.Get("settings.thermal.on"), rows[Loc.Get("settings.thermal")]);
    }

    [Fact]
    public void Hardware_rows_skip_what_could_not_be_read()
    {
        var labels = HardwareText.Rows(Minimal).Select(r => r.Label).ToList();

        Assert.Equal([Loc.Get("settings.device"), "Disk", Loc.Get("settings.thermal")], labels);
    }

    // ---- 卡顿诊断 ----

    [Fact]
    public void Diagnosis_names_the_cause_and_lists_top_users()
    {
        var v = DiagnoseText.Build(TestSupport.Fixture<StutterReport>("stutter_report.json"));

        Assert.Equal(Loc.Format("diagnose.background_cpu", "OneDrive"), v.Conclusion);
        Assert.Equal(["OneDrive  12%", "Chrome  5%"], v.TopCpu);
        Assert.Equal(["Chrome  3.0 GB"], v.TopMemory);
    }
}
