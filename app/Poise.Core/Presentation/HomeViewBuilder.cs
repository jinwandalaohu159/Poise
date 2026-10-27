using Poise.Core.Ipc;
using Poise.Core.Localization;
using Status = Poise.Core.Ipc.Status;

namespace Poise.Core.Presentation;

public sealed record MetricView(string Label, double? Value, Func<double, string> Format, Level Level, string Tooltip);

/// <summary>主页、托盘面板、托盘图标需要显示的一切。</summary>
public sealed record HomeView(
    bool Connected,
    Level? Level,
    string Title,
    string Description,
    bool DryRun,
    MetricView Cpu,
    MetricView Gpu,
    MetricView Memory,
    Mode Selected,
    Mode Effective,
    bool AutoFullForGames,
    bool AutoClean,
    /// <summary>"建议重启"时可以选"稍后提醒"。</summary>
    bool CanSnoozeRestart,
    /// <summary>自动清理一行的说明（上次清理的结果，或功能说明）。</summary>
    string AutoCleanDetail);

/// <summary>把服务状态换算成状态文案和指标（纯换算，不发请求）。</summary>
public static class HomeViewBuilder
{
    private const float CpuBusyPct = 90;
    private const float MemoryNoticePct = 80;
    private const float MemoryBadPct = 92;
    private const float WarmCelsius = 80;
    private const float HotCelsius = 90;

    public static HomeView Build(Status? status, DateTimeOffset now)
    {
        if (status is null)
        {
            var none = new MetricView("", null, v => v.ToString("0"), Level.Good, "");
            return new HomeView(false, null, Loc.Get("status.disconnected"), Loc.Get("status.disconnected.desc"), false,
                none with { Label = Loc.Get("metric.cpu_load") }, none with { Label = Loc.Get("metric.gpu_temp") },
                none with { Label = Loc.Get("metric.mem") }, Mode.Balanced, Mode.Balanced, true, true,
                false, Loc.Get("home.auto_clean.desc"));
        }
        var (title, description) = Describe(status);
        return new HomeView(true, status.Health.Level, title, description, status.DryRun,
            CpuMetric(status.Metrics), GpuMetric(status.Metrics), MemoryMetric(status.Metrics),
            status.Mode, status.EffectiveMode, status.AutoFullForGames, status.AutoClean,
            status.Health.Code == HealthCode.RestartRecommended,
            CleanupText.AutoCleanDetail(status.LastCleanup, now));
    }

    private static (string, string) Describe(Status s) => s.Health.Code switch
    {
        HealthCode.InGame => (Loc.Get("status.in_game"),
            Loc.Format(s.EffectiveMode == Mode.Full ? "status.in_game.desc" : "status.in_game.desc_no_full", s.Health.Game)),
        HealthCode.Tired => (Loc.Get("status.tired"), Loc.Get("status.tired.desc")),
        HealthCode.Hot => (Loc.Get("status.hot"), Loc.Get("status.hot.desc")),
        HealthCode.RestartRecommended => (Loc.Get("status.restart"), Loc.Get("status.restart.desc")),
        HealthCode.StorageLow => (Loc.Get("status.storage"), Loc.Get("status.storage.desc")),
        _ => (Loc.Get("status.smooth"), Loc.Get("status.smooth.desc")),
    };

    /// <summary>有 CPU 温度时显示温度，否则显示占用。</summary>
    private static MetricView CpuMetric(Metrics m) =>
        m.CpuTemp is { } t
            ? Temperature(Loc.Get("metric.cpu_temp"), t)
            : new MetricView(Loc.Get("metric.cpu_load"), m.CpuPct, v => Format.Percent((float)v),
                m.CpuPct >= CpuBusyPct ? Level.Notice : Level.Good,
                $"{Loc.Get("metric.cpu_load")} {Format.Percent(m.CpuPct)}");

    private static MetricView GpuMetric(Metrics m) =>
        m.GpuTemp is { } t
            ? Temperature(Loc.Get("metric.gpu_temp"), t)
            : new MetricView(Loc.Get("metric.gpu_temp"), null, v => v.ToString("0"), Level.Good, "");

    private static MetricView MemoryMetric(Metrics m)
    {
        var level = m.MemUsedPct >= MemoryBadPct ? Level.Bad : m.MemUsedPct >= MemoryNoticePct ? Level.Notice : Level.Good;
        var tip = $"{Loc.Get("metric.mem")} {Format.Bytes(m.MemUsedBytes)} / {Format.Bytes(m.MemTotalBytes)}";
        return new MetricView(Loc.Get("metric.mem"), m.MemUsedPct, v => Format.Percent((float)v), level, tip);
    }

    private static MetricView Temperature(string label, float celsius)
    {
        var level = celsius >= HotCelsius ? Level.Bad : celsius >= WarmCelsius ? Level.Notice : Level.Good;
        var word = Loc.Get(level switch { Level.Bad => "metric.hot", Level.Notice => "metric.warm", _ => "metric.normal" });
        return new MetricView(label, celsius, v => Format.Temperature((float)v), level, Loc.Format("metric.temp_tooltip", label, celsius, word));
    }
}
