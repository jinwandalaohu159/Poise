using Poise.Core.Ipc;
using Poise.Core.Localization;
using Status = Poise.Core.Ipc.Status;

namespace Poise.Core.Presentation;

/// <summary>
/// 一个指标：数值（不含单位）、单位、比例条的长度（0～1）、等级与悬停说明。读不到时 <see cref="Value"/> 为 null。
/// </summary>
public sealed record MetricView(string Label, double? Value, string Unit, double? Fraction, Level Level, string Tooltip);

/// <summary>
/// 主页清理模块卡：可释放多少、几项需要处理、上次清理；
/// 内存条显示已用比例，其中可释放的部分单独标出。
/// </summary>
public sealed record CleanupCard(ulong FreeableBytes, int Items, string Detail, double MemoryFraction, double FreeableFraction);

/// <summary>
/// 风扇：<see cref="Speed"/> 为"当前 3,800 / 4,800 RPM"（各风扇平均 / 最高转速；界面隐藏时服务不读转速，此时为空）；
/// <see cref="Boost"/> 为加速设置（0 = 自动，由 BIOS 按温度调节）。
/// </summary>
public sealed record FanView(string Speed, int Boost)
{
    /// <summary>滑块旁的数值：自动 / 65%。</summary>
    public static string BoostText(int boost) => boost > 0 ? $"{boost}%" : Loc.Get("home.fan.auto");
}

/// <summary>
/// 主页上少见的提醒：需要重启（可以"稍后提醒"）优先，其次是观察模式。
/// <see cref="Level"/> 为 Notice 时按警告显示，否则按一般信息显示。
/// </summary>
public sealed record HomeNotice(Level Level, string Title, string Message, bool CanSnooze);

/// <summary>主页与托盘面板需要显示的一切。</summary>
public sealed record HomeView(
    bool Connected,
    bool DryRun,
    MetricView CpuLoad,
    MetricView CpuTemp,
    MetricView GpuTemp,
    MetricView Memory,
    /// <summary>用户选择的模式（可能是"自动"）。</summary>
    Mode Selected,
    /// <summary>此刻实际使用的模式（"自动"换算后、游戏时切换后、过热时降档后）。</summary>
    Mode Effective,
    /// <summary>要标"使用中"的模式；没有时为 null。见 <see cref="ModeText.InUse"/>。</summary>
    Mode? InUse,
    /// <summary>机型极限模式的标识（例如 "dell_g_mode"）；不支持时为 null。</summary>
    string? VendorMode,
    GameMode GameMode,
    /// <summary>"建议重启"时可以选"稍后提醒"。</summary>
    bool CanSnoozeRestart,
    /// <summary>主页顶部的提醒；没有时为 null。</summary>
    HomeNotice? Notice,
    CleanupCard Cleanup,
    /// <summary>本机不支持风扇加速或未连接时为 null（不显示风扇一行）。</summary>
    FanView? Fan)
{
    internal const string CelsiusUnit = "°C";
}

/// <summary>把服务状态换算成主页的指标、模式与模块卡（纯换算，不发请求）。</summary>
public static class HomeViewBuilder
{
    private const float CpuBusyPct = 90;
    private const float MemoryNoticePct = 80;
    private const float MemoryBadPct = 92;
    private const float WarmCelsius = 80;
    private const float HotCelsius = 90;
    /// <summary>温度比例条的满刻度（处理器的降频温度在 100°C 左右）。</summary>
    private const float TempScale = 100;

    public static HomeView Build(Status? status, DateTimeOffset now)
    {
        if (status is null)
        {
            MetricView None(string key, string unit) => new(Loc.Get(key), null, unit, null, Level.Good, "");
            return new HomeView(false, false,
                None("metric.cpu_load", "%"), None("metric.cpu_temp", HomeView.CelsiusUnit), None("metric.gpu_temp", HomeView.CelsiusUnit), None("metric.mem", "%"),
                Mode.Auto, Mode.Balanced, null, null, GameMode.Full, false, null,
                new CleanupCard(0, 0, Loc.Get("home.auto_clean.desc"), 0, 0), null);
        }
        var m = status.Metrics;
        var restart = status.Health.Code == HealthCode.RestartRecommended;
        return new HomeView(true, status.DryRun,
            CpuLoad(m), Temperature("metric.cpu_temp", m.CpuTemp), Temperature("metric.gpu_temp", m.GpuTemp), MemoryMetric(m),
            status.Mode, status.EffectiveMode, ModeText.InUse(status), status.Hardware.VendorMode, status.GameMode,
            restart, Notice(restart, status.DryRun),
            CleanupSummary(status, now),
            Fan(status));
    }

    private static HomeNotice? Notice(bool restart, bool dryRun) =>
        restart ? new HomeNotice(Level.Notice, Loc.Get("status.restart"), Loc.Get("status.restart.desc"), true)
        : dryRun ? new HomeNotice(Level.Good, Loc.Get("status.observe"), Loc.Get("status.observe.desc"), false)
        : null;

    private static FanView? Fan(Status s)
    {
        if (!s.Hardware.FanBoostSupported)
        {
            return null;
        }
        var speed = FanText.Speed(s.Hardware.Fans) is { } value ? Loc.Format("home.fan.speed", value) : "";
        return new FanView(speed, s.FanBoost);
    }

    /// <summary>说明行：几项需要处理，以及上次清理的结果。</summary>
    private static CleanupCard CleanupSummary(Status s, DateTimeOffset now)
    {
        var items = s.Health.Cleanable;
        var head = items > 0 ? Loc.Format("home.cleanup.items", items) : Loc.Get("home.cleanup.none");
        var detail = s.LastCleanup is null ? head : $"{head} · {CleanupText.AutoCleanDetail(s.LastCleanup, now)}";
        double total = s.Metrics.MemTotalBytes;
        var freeable = total > 0 ? Math.Min(s.Health.FreeableBytes, s.Metrics.MemUsedBytes) / total : 0;
        return new CleanupCard(s.Health.FreeableBytes, items, detail, s.Metrics.MemUsedPct / 100.0, freeable);
    }

    private static MetricView CpuLoad(Metrics m)
    {
        var label = Loc.Get("metric.cpu_load");
        return new MetricView(label, m.CpuPct, "%", m.CpuPct / 100,
            m.CpuPct >= CpuBusyPct ? Level.Notice : Level.Good,
            $"{label} {Format.Percent(m.CpuPct)}");
    }

    private static MetricView MemoryMetric(Metrics m)
    {
        var label = Loc.Get("metric.mem");
        var level = m.MemUsedPct >= MemoryBadPct ? Level.Bad : m.MemUsedPct >= MemoryNoticePct ? Level.Notice : Level.Good;
        var tip = $"{label} {Format.Bytes(m.MemUsedBytes)} / {Format.Bytes(m.MemTotalBytes)}";
        return new MetricView(label, m.MemUsedPct, "%", m.MemUsedPct / 100, level, tip);
    }

    private static MetricView Temperature(string key, float? celsius)
    {
        var label = Loc.Get(key);
        if (celsius is not { } t)
        {
            return new MetricView(label, null, HomeView.CelsiusUnit, null, Level.Good, "");
        }
        var level = t >= HotCelsius ? Level.Bad : t >= WarmCelsius ? Level.Notice : Level.Good;
        var word = Loc.Get(level switch { Level.Bad => "metric.hot", Level.Notice => "metric.warm", _ => "metric.normal" });
        return new MetricView(label, t, HomeView.CelsiusUnit, Math.Clamp(t / TempScale, 0, 1), level, Loc.Format("metric.temp_tooltip", label, t, word));
    }
}
