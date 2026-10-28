using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>系统优化的进度：只统计推荐项（一键优化会处理的项目）。</summary>
public sealed record OptimizeSummary(int Optimized, int Total)
{
    public double Fraction => Total > 0 ? (double)Optimized / Total : 1;

    public int Remaining => Total - Optimized;

    /// <summary>"已优化 4 / 6 项"。</summary>
    public string Text => Loc.Format("optimize.summary", Optimized, Total);

    /// <summary>"2 项可优化" / "推荐项均已优化"。</summary>
    public string Detail => Remaining > 0 ? Loc.Format("home.optimize.remaining", Remaining) : Loc.Get("home.optimize.all");

    public static OptimizeSummary From(IEnumerable<ItemStatus> items)
    {
        var recommended = items.Where(i => i.Item.InOneClick).ToList();
        return new OptimizeSummary(recommended.Count(i => i.IsOptimized), recommended.Count);
    }
}
