using Poise.Core.Ipc;

namespace Poise.Core.Monitoring;

/// <summary>
/// 走势上的一个点，时刻已换算到界面的本地时钟。
/// <see cref="Performance"/>、<see cref="Efficiency"/> 是性能核、能效核的平均占用，不是大小核处理器时为 null。
/// </summary>
public readonly record struct TrendSample(TimeSpan At, float Cpu, float? Performance, float? Efficiency, float Memory, float? CpuTemp)
{
    /// <summary>判断高负载用的值：性能核负载（前台程序跑在性能核上）；不是大小核处理器时为整机 CPU。</summary>
    public float Load => Performance ?? Cpu;
}

/// <summary>
/// 最近几分钟的走势。服务给出的是"距今多少秒"，收到时换算成本地时钟上的时刻：
/// 曲线位置只取决于本地时钟，两次刷新之间可以连续平移，与请求的快慢、两边节拍是否对齐无关。
/// </summary>
public sealed class TrendSeries
{
    /// <summary>
    /// 显示的"现在"比实际晚这么多：服务每 2 秒采样一次，界面每 2 秒读取一次，
    /// 最新的点最多晚约 4 秒。这样曲线右端始终有数据，不会时有时无。
    /// </summary>
    public static readonly TimeSpan DisplayDelay = TimeSpan.FromSeconds(4);

    /// <summary>覆盖不到这么久时提示"数据收集中"。</summary>
    private static readonly TimeSpan FewData = TimeSpan.FromMinutes(1);

    public static readonly TrendSeries Empty = new(TimeSpan.FromMinutes(5), []);

    public TrendSeries(TimeSpan window, IReadOnlyList<TrendSample> samples)
    {
        Window = window > TimeSpan.Zero ? window : TimeSpan.FromMinutes(5);
        Samples = samples;
    }

    /// <summary>横轴覆盖的时长。</summary>
    public TimeSpan Window { get; }

    /// <summary>从早到晚。</summary>
    public IReadOnlyList<TrendSample> Samples { get; }

    /// <summary>是否有性能核、能效核两条线（否则只有一条整机 CPU 线）。</summary>
    public bool Hybrid => Samples.Any(s => s.Performance is not null);

    public TrendSample? Latest => Samples.Count > 0 ? Samples[^1] : null;

    /// <summary>数据还不到 1 分钟（刚启动），走势图大部分是空的。</summary>
    public bool Collecting => Samples.Count < 2 || Samples[^1].At - Samples[0].At < FewData;

    /// <param name="receivedAt">收到回复时本地时钟的读数。</param>
    public static TrendSeries From(Trend trend, TimeSpan receivedAt) => new(
        TimeSpan.FromSeconds(trend.WindowSecs),
        trend.Points
            .Select(p => new TrendSample(
                receivedAt - TimeSpan.FromSeconds(p.SecsAgo),
                p.CpuPct, p.PerformancePct, p.EfficiencyPct, p.MemPct, p.CpuTemp))
            .ToList());

    /// <summary>
    /// 在本地时刻 <paramref name="now"/>、宽 <paramref name="width"/> 的图上，时刻 <paramref name="at"/> 的横坐标。
    /// 超出 0～width 的点由图裁掉，不压到边上。
    /// </summary>
    public double X(TimeSpan at, TimeSpan now, double width) =>
        width * (1 - (now - DisplayDelay - at).TotalSeconds / Window.TotalSeconds);
}
