using System.Globalization;
using Poise.Core.Localization;

namespace Poise.Core;

/// <summary>面向用户的数字与时间格式：只显示有用的精度。</summary>
public static class Format
{
    public static string Bytes(ulong bytes)
    {
        var (value, unit) = BytesParts(bytes);
        return $"{value} {unit}";
    }

    /// <summary>同 <see cref="Bytes"/>，数字和单位分开（大数字旁边配小号的单位）。</summary>
    public static (string Value, string Unit) BytesParts(ulong bytes)
    {
        const double gb = 1024d * 1024 * 1024;
        const double mb = 1024d * 1024;
        return bytes >= gb
            ? ((bytes / gb).ToString("0.0", CultureInfo.InvariantCulture), "GB")
            : ((bytes / mb).ToString("0", CultureInfo.InvariantCulture), "MB");
    }

    /// <summary>
    /// 硬件容量：整数不带小数（16 GB、780 GB），1 TB 起用 TB（1.8 TB）。
    /// 与 <see cref="Bytes"/> 的区别：容量是固定规格，不需要固定一位小数来对齐变化。
    /// </summary>
    public static string Capacity(ulong bytes)
    {
        const double tb = 1024d * 1024 * 1024 * 1024;
        const double gb = 1024d * 1024 * 1024;
        const double mb = 1024d * 1024;
        return bytes switch
        {
            >= (ulong)tb => Trim(bytes / tb) + " TB",
            >= (ulong)gb => (bytes / gb >= 100 ? (bytes / gb).ToString("0", CultureInfo.InvariantCulture) : Trim(bytes / gb)) + " GB",
            _ => (bytes / mb).ToString("0", CultureInfo.InvariantCulture) + " MB",
        };

        static string Trim(double v) => v.ToString("0.#", CultureInfo.InvariantCulture);
    }

    /// <summary>个数：中文过万用"万"（4.6 万），英文过万用 K（46K），其余带千分位。</summary>
    public static string Count(ulong n)
    {
        var chinese = Loc.Language == Loc.Chinese;
        var unit = chinese ? 10_000d : 1_000d;
        if (n < 10_000)
        {
            return n.ToString("N0", CultureInfo.InvariantCulture);
        }
        var value = (n / unit).ToString(n / unit >= 100 ? "0" : "0.#", CultureInfo.InvariantCulture);
        return chinese ? $"{value} 万" : $"{value}K";
    }

    /// <summary>一段时长：3 天 4 小时 / 5 小时 12 分钟 / 8 分钟。</summary>
    public static string Duration(TimeSpan d)
    {
        if (d.TotalDays >= 1)
        {
            return Loc.Format("time.span.days", (int)d.TotalDays, d.Hours);
        }
        return d.TotalHours >= 1
            ? Loc.Format("time.span.hours", (int)d.TotalHours, d.Minutes)
            : Loc.Format("time.span.minutes", Math.Max(1, (int)d.TotalMinutes));
    }

    /// <summary>温度（紧凑写法，几个并列时用）："62°"。</summary>
    public static string Temperature(float celsius) => $"{celsius:0}°";

    /// <summary>温度（单独一项时用）："62°C"。</summary>
    public static string Celsius(float celsius) => $"{celsius:0}°C";

    public static string Percent(float pct) => $"{pct:0}%";

    /// <summary>占整机的份额，保留一位小数（任务管理器的口径）："1.2%"。</summary>
    public static string Share(double pct) => $"{pct:0.0}%";

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
