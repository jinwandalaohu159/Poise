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
        Assert.True(s.AutoFullForGames);
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
        Assert.Equal(("WD SN750 2TB", 45u, DiskHealth.Warning, 3u), (disk.Name, disk.TempC, disk.Health, disk.WearPct));
        Assert.Equal(12UL << 30, hw.SystemDrive?.FreeBytes);
        Assert.Equal((32UL << 30, 4800u, 2u), (hw.Memory!.TotalBytes, hw.Memory.SpeedMhz, hw.Memory.Modules));

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
        Assert.Equal(("微信", 2UL << 30), (r.Advisories[0].Name, r.Advisories[0].PrivateBytes!.Value));
        Assert.Equal(2, r.Advisories[2].Count);
    }

    [Fact]
    public void Reads_a_stutter_report()
    {
        var r = TestSupport.Fixture<StutterReport>("stutter_report.json");

        Assert.Equal("background_cpu", r.CauseKind);
        Assert.Equal("OneDrive", r.CauseName);
        Assert.Equal("OneDrive", r.TopCpu[0][0].GetString());
        Assert.Equal(12.0, r.TopCpu[0][1].GetDouble());
        Assert.Equal(3UL << 30, r.TopMemory[0][1].GetUInt64());
    }

    [Fact]
    public void Reads_the_error_code_of_a_failed_response()
    {
        using var doc = JsonDocument.Parse(TestSupport.FixtureText("error_response.json"));

        Assert.False(doc.RootElement.GetProperty("ok").GetBoolean());
        Assert.Equal(ErrorCode.ObserveOnly, PoiseClient.ParseCode(doc.RootElement));
    }

    [Theory]
    [InlineData("""{"id":1,"ok":false,"code":"from_a_newer_service"}""")]
    [InlineData("""{"id":1,"ok":false}""")]
    [InlineData("""{"id":1,"ok":false,"code":3}""")]
    public void Unrecognized_error_codes_are_unknown(string json)
    {
        using var doc = JsonDocument.Parse(json);

        Assert.Equal(ErrorCode.Unknown, PoiseClient.ParseCode(doc.RootElement));
    }
}
