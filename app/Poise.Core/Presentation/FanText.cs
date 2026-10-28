using System.Globalization;
using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>风扇转速的显示：当前转速（多个风扇取平均）/ 最高转速，例如 "3,935 / 4,800 RPM"。主页与设置页共用。</summary>
public static class FanText
{
    /// <summary>没有风扇读数时为 null。</summary>
    public static string? Speed(IReadOnlyList<FanInfo> fans) =>
        fans.Count == 0
            ? null
            : Loc.Format("hw.fan_value", Count((ulong)fans.Average(Shown)), Count(fans.Max(f => f.MaxRpm)));

    /// <summary>
    /// 全速时读数会短暂略超标称最高转速（如 4,819 / 4,800），显示时以最高转速为上限，避免出现"当前大于最高"。
    /// 读不到最高转速时原样显示。
    /// </summary>
    private static double Shown(FanInfo f) => f.MaxRpm > 0 ? Math.Min(f.Rpm, f.MaxRpm) : f.Rpm;

    private static string Count(ulong n) => n.ToString("N0", CultureInfo.InvariantCulture);
}
