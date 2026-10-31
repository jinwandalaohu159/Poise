using System.Text.Json;
using Poise.Core.Ipc;

namespace Poise.Core.Tests;

/// <summary>
/// 服务（Rust）生成的样本能被界面正确读取。样本由 poise-protocol 的 contract 测试生成，
/// 两边任何一方改了字段名、枚举值或可空性，这里或那边就会失败。
/// </summary>
public sealed class ContractTests
{
    [Fact]
    public void Reads_a_status_with_every_field_set()
    {
        var s = TestSupport.Fixture<Status>("status_full.json");

        Assert.Equal(HealthCode.StorageLow, s.Health.Code);
        Assert.Equal(Level.Notice, s.Health.Level);
        Assert.Equal("League of Legends", s.Health.Game);
        Assert.Equal(2, s.Health.Cleanable);
        Assert.Equal(Mode.Quiet, s.Mode);
        Assert.Equal(Mode.Full, s.EffectiveMode);
        Assert.Equal(GameMode.Vendor, s.GameMode);
        Assert.Equal(60, s.FanBoost);
        Assert.Equal(("dell_g_mode", true), (s.Hardware.VendorMode, s.Hardware.FanBoostSupported));
        Assert.False(s.AutoClean);
        Assert.True(s.DryRun);
        Assert.Equal(7, s.AdjustedProcesses);
        Assert.Equal(1, s.DeferredCleanup);

        Assert.Equal(71f, s.Metrics.CpuTemp);
        Assert.Equal(58f, s.Metrics.GpuTemp);
        Assert.Equal(32UL << 30, s.Metrics.MemTotalBytes);
        Assert.True(s.Metrics.OnBattery);

        var hw = s.Hardware;
        Assert.Equal("Dell G15 5520", hw.Profile);
        Assert.True(hw.ThermalControl);
        Assert.Equal([2400u, 2600u], hw.Fans.Select(f => f.Rpm));
        Assert.All(hw.Fans, f => Assert.Equal(4800u, f.MaxRpm));
        var disk = Assert.Single(hw.Disks);
        Assert.Equal(("WD SN750 2TB", 2000UL << 30, 45u, DiskHealth.Warning, 3u), (disk.Name, disk.SizeBytes, disk.TempC, disk.Health, disk.WearPct));
        Assert.Equal(new VolumeInfo("C", 12UL << 30, 500UL << 30), Assert.Single(disk.Volumes));
        Assert.Equal(12UL << 30, hw.SystemDrive?.FreeBytes);
        Assert.Equal((32UL << 30, 4800u), (hw.Memory!.TotalBytes, hw.Memory.SpeedMhz));
        Assert.Equal([16UL << 30, 16UL << 30], hw.Memory.ModuleBytes);
        Assert.Equal(new CpuInfo(6, 12, 8, 8), hw.Cpu);
        Assert.True(s.SmartSchedule);

        var last = s.LastCleanup!;
        Assert.Equal(CleanupTrigger.Deferred, last.Trigger);
        Assert.Equal(1_800_000_000_000UL, last.AtUnixMs);
        Assert.Equal(2UL << 30, last.FreedBytes);
        Assert.Equal(3, last.Handled);
    }

    [Fact]
    public void Reads_a_status_with_optional_fields_empty()
    {
        var s = TestSupport.Fixture<Status>("status_minimal.json");

        Assert.Equal(HealthCode.Smooth, s.Health.Code);
        Assert.Null(s.Health.Game);
        Assert.Null(s.Metrics.CpuTemp);
        Assert.Null(s.Metrics.GpuTemp);
        Assert.Empty(s.Hardware.Fans);
        Assert.Null(s.Hardware.SystemDrive);
        Assert.Null(s.Hardware.Memory);
        Assert.Null(s.LastCleanup);
        var disk = Assert.Single(s.Hardware.Disks);
        Assert.Equal((null, DiskHealth.Unknown, null), ((uint?)disk.TempC, disk.Health, (uint?)disk.WearPct));
        Assert.Null(disk.SizeBytes);
        Assert.Empty(disk.Volumes);
    }

    [Fact]
    public void Reads_a_cleanup_result_with_every_advisory_kind()
    {
        var r = TestSupport.Fixture<CleanupResult>("cleanup_result.json");

        Assert.Equal(1100UL << 20, r.FreedBytes);
        Assert.Single(r.Handled);
        Assert.Single(r.Failed);
        Assert.Equal(1, r.Deferred);
        Assert.Equal(["heavy_app", "restart_recommended", "manual_pending"], r.Advisories.Select(a => a.Kind));
        Assert.Equal(("微信", 2UL << 30), (r.Advisories[0].Name, r.Advisories[0].MemoryBytes!.Value));
        Assert.Equal(2, r.Advisories[2].Count);
    }

    [Fact]
    public void Reads_a_diagnosis()
    {
        var d = TestSupport.Fixture<Diagnosis>("diagnosis.json");

        Assert.Equal("background_cpu", d.CauseKind);
        Assert.Equal("OneDrive", d.CauseName);
        Assert.EndsWith("onedrive.exe", d.CausePath);
        Assert.Equal(35f, d.CauseShare);
        Assert.Equal((92f, 70f, 82f, false), (d.CpuPct, d.MemUsedPct, d.CpuTemp!.Value, d.Throttling));
        Assert.Equal("OneDrive", d.TopCpu[0][0].GetString());
        Assert.Equal(3UL << 30, d.TopMemory[0][1].GetUInt64());
        Assert.True(d.SmartSchedule);
    }

    [Fact]
    public void Reads_a_trend_with_core_loads()
    {
        var t = TestSupport.Fixture<Trend>("trend.json");

        Assert.Equal(300u, t.WindowSecs);
        Assert.Equal([4.5f, 2f], t.Points.Select(p => p.SecsAgo));
        Assert.Equal((64f, 18.5f), (t.Points[0].PerformancePct!.Value, t.Points[0].EfficiencyPct!.Value));
        Assert.Null(t.Points[1].PerformancePct);
        Assert.Null(t.Points[1].CpuTemp);
    }

    [Fact]
    public void Reads_apps_with_their_processes()
    {
        var browser = TestSupport.Fixture<List<AppEntry>>("apps.json")[1];

        Assert.Equal((6.5f, 3UL << 30), (browser.CpuPct, browser.MemoryBytes));
        Assert.Equal(["browser.exe", "gpu-helper.exe"], browser.Processes.Select(p => p.Name));
        Assert.Equal(ScheduleState.SelfManaged, browser.Processes[1].Schedule.State);
        // 主进程跟随程序；有自己窗口的另一个程序可以单独设置类别。
        Assert.Null(browser.Processes[0].Preference);
        Assert.Equal(AppPreference.Game, browser.Processes[1].Preference);
        Assert.Equal(@"c:\apps\helper\gpu-helper.exe", browser.Processes[1].Path);
        Assert.Equal(1UL << 30, browser.Processes[1].MemoryBytes);
    }

    [Fact]
    public void Reads_apps_with_every_schedule_state()
    {
        var apps = TestSupport.Fixture<List<AppEntry>>("apps.json");

        Assert.Equal(new AppSchedule(ScheduleState.Scheduled, false, CoreUse.Efficiency, PriorityLevel.Lower, Adjustment.Yielded), apps[0].Schedule);
        Assert.Equal(new AppSchedule(ScheduleState.Scheduled, true, CoreUse.Performance, PriorityLevel.Higher, Adjustment.Raised), apps[1].Schedule);
        Assert.Null(apps[2].Schedule!.Cores);
        Assert.Equal(PriorityLevel.High, apps[2].Schedule!.Priority);
        Assert.Equal([ScheduleState.Excluded, ScheduleState.SelfManaged, ScheduleState.Off], apps.Skip(3).Take(3).Select(a => a.Schedule!.State));
        Assert.Equal([AppKind.App, AppKind.App, AppKind.App, AppKind.Excluded], apps.Take(4).Select(a => a.Kind));
        Assert.Null(apps[6].Schedule);
        Assert.False(apps[6].Running);
    }

    [Fact]
    public void Reads_a_cleanup_overview()
    {
        var o = TestSupport.Fixture<CleanupOverview>("cleanup_overview.json");

        Assert.Equal((3, true, true), (o.NeedsAttention, o.RestartAdvised, o.AutoClean));
        Assert.Null(o.LastCleanup);
        Assert.Equal(4, o.Processes.Count);
        var audio = o.Processes[0];
        Assert.Equal((ProcessIssue.Oversized, "restart_service", "Audiosrv", true),
            (audio.Issue, audio.Remedy.Kind, audio.Remedy.Service, audio.Disruptive));
        var wechat = o.Processes[1];
        Assert.Equal((ProcessIssue.Growing, 80UL << 20, 12, "reopen_manually"),
            (wechat.Issue, wechat.GrowthPerHour!.Value, wechat.ProcessCount, wechat.Remedy.Kind));
        Assert.Null(wechat.Path);
        Assert.Equal((ProcessIssue.None, "none"), (o.Processes[2].Issue, o.Processes[2].Remedy.Kind));
        Assert.Null(o.Processes[2].SinceStart);
        var push = o.Processes[3];
        Assert.Equal((ProcessIssue.Accumulated, new Change(GrowthMetric.Handles, 2_100, 46_000)), (push.Issue, push.SinceStart));

        var boot = o.SinceBoot!;
        Assert.Equal((300_000UL, 259_200_000UL), (boot.BaselineUptimeMs, boot.UptimeMs));
        Assert.Equal([TotalItem.Handles, TotalItem.KernelPool, TotalItem.SystemCache], boot.Items.Select(i => i.Item));
        Assert.Equal(new TotalChange(TotalItem.Handles, 82_000, 168_000, true), boot.Items[0]);
    }

    [Fact]
    public void Reads_the_error_code_of_a_failed_response()
    {
        using var doc = JsonDocument.Parse(TestSupport.FixtureText("error_response.json"));

        Assert.False(doc.RootElement.GetProperty("ok").GetBoolean());
        Assert.Equal(ErrorCode.ObserveOnly, CodeOf(doc.RootElement));
    }

    [Theory]
    [InlineData("""{"id":1,"ok":false,"code":"from_a_newer_service"}""")]
    [InlineData("""{"id":1,"ok":false}""")]
    [InlineData("""{"id":1,"ok":false,"code":3}""")]
    public void Unrecognized_error_codes_are_unknown(string json)
    {
        using var doc = JsonDocument.Parse(json);

        Assert.Equal(ErrorCode.Unknown, CodeOf(doc.RootElement));
    }

    /// <summary>整条回复里的错误代码。</summary>
    private static ErrorCode CodeOf(JsonElement reply) =>
        PipeChannel.ParseCode(reply.TryGetProperty("code", out var code) ? code : default);
}
