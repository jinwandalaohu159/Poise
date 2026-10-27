using System.Globalization;
using Poise.Core.Localization;

namespace Poise.Core;

/// <summary>面向用户的数字与时间格式：只显示有用的精度。</summary>
public static class Format
{
    public static string Bytes(ulong bytes)
    {
        const double gb = 1024d * 1024 * 1024;
        const double mb = 1024d * 1024;
        return bytes >= gb
            ? (bytes / gb).ToString("0.0", CultureInfo.InvariantCulture) + " GB"
            : (bytes / mb).ToString("0", CultureInfo.InvariantCulture) + " MB";
    }

    public static string Temperature(float celsius) => $"{celsius:0}°";

    public static string Percent(float pct) => $"{pct:0}%";

    /// <summary>某个时刻（按本地时间）：今天 14:02 / 昨天 14:02 / 10月25日。</summary>
    public static string When(DateTimeOffset at, DateTimeOffset now)
    {
        var local = at.ToLocalTime();
        var today = now.ToLocalTime().Date;
        var time = local.ToString("HH:mm", CultureInfo.InvariantCulture);
        if (local.Date == today)
        {
            return Loc.Format("time.today", time);
        }
        if (local.Date == today.AddDays(-1))
        {
            return Loc.Format("time.yesterday", time);
        }
        return Loc.Format("time.date", local.Month, local.Day);
    }
}
