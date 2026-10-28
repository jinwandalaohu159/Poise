using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>"与开机时相比"的一行：项目、开机时、现在、变化。Notable 的行用提醒色。</summary>
public sealed record SinceBootRow(string Label, string AtBoot, string Now, string Change, bool Notable);

/// <summary>
/// "与开机时相比"卡片。还没有基线时 Rows 为空，Caption 说明何时开始记录。
/// </summary>
public sealed record SinceBootView(string Caption, IReadOnlyList<SinceBootRow> Rows, int NotableCount);

/// <summary>整机总量与开机基线的对比文案。</summary>
public static class SinceBootText
{
    public static SinceBootView Build(SinceBoot? s, DateTimeOffset now)
    {
        if (s is null)
        {
            return new SinceBootView(Loc.Get("since_boot.pending"), [], 0);
        }
        var caption = Loc.Format("since_boot.caption",
            Format.Duration(TimeSpan.FromMilliseconds(s.UptimeMs)),
            Format.When(DateTimeOffset.FromUnixTimeMilliseconds((long)s.BaselineAtUnixMs), now));
        var rows = s.Items.Select(Row).ToList();
        return new SinceBootView(caption, rows, rows.Count(r => r.Notable));
    }

    internal static SinceBootRow Row(TotalChange c)
    {
        Func<ulong, string> format = IsBytes(c.Item) ? Format.Bytes : Format.Count;
        var (sign, delta) = c.Now >= c.AtBoot ? ("+", c.Now - c.AtBoot) : ("−", c.AtBoot - c.Now);
        return new SinceBootRow(Loc.Get(Key(c.Item)), format(c.AtBoot), format(c.Now), sign + format(delta), c.Notable);
    }

    private static bool IsBytes(TotalItem item) =>
        item is TotalItem.Commit or TotalItem.KernelPool or TotalItem.SystemCache;

    private static string Key(TotalItem item) => item switch
    {
        TotalItem.Handles => "total.handles",
        TotalItem.Threads => "total.threads",
        TotalItem.Processes => "total.processes",
        TotalItem.Commit => "total.commit",
        TotalItem.KernelPool => "total.kernel_pool",
        _ => "total.system_cache",
    };
}
