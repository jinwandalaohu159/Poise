using Poise.Core.Localization;
using Poise.Core.Monitoring;

namespace Poise.Core.Presentation;

/// <summary>走势卡图下方的当前负载。</summary>
public static class TrendText
{
    /// <summary>"性能核 64% · 能效核 18%"；不是大小核处理器时为"CPU 占用 20%"；没有数据时为空。</summary>
    public static string Loads(TrendSample? s) => s switch
    {
        null => "",
        { Performance: { } p, Efficiency: { } e } => Loc.Format("trend.loads.hybrid", Math.Round(p), Math.Round(e)),
        { } cpu => Loc.Format("trend.loads.cpu", Math.Round(cpu.Cpu)),
    };
}
