using System.Globalization;
using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>
/// 这台电脑的情况，决定核心的叫法：大小核处理器说"大核""小核"，其余（多 CCD 等）说"主核心组""其他核心组"；
/// Vendor 是机型极限模式的标识（模式名称用）。
/// </summary>
public sealed record Machine(bool Hybrid, string? Vendor)
{
    public static Machine From(Status? s) =>
        new(s?.Hardware.Cpu is { EfficiencyCores: > 0 }, s?.Hardware.VendorMode);
}

/// <summary>核心与缓存面板、日志、使用记录共用的说法：程序为什么在那里、用哪些核心、负载、缓存容量。</summary>
public static class CoreWording
{
    /// <summary>对程序的决定的简称（标签上显示）。</summary>
    public static string Role(Verdict v) => Loc.Get(v switch
    {
        Verdict.Playing => "verdict.playing",
        Verdict.Focused => "verdict.focused",
        Verdict.Yielded => "verdict.yielded",
        Verdict.Lowered => "verdict.lowered",
        Verdict.Backseat => "verdict.backseat",
        Verdict.Aside => "verdict.aside",
        _ => "verdict.normal",
    });

    /// <summary>重点程序用提示色，其余不着色。</summary>
    public static TagTone Tone(Verdict v) => v is Verdict.Playing or Verdict.Focused ? TagTone.Info : TagTone.None;

    /// <summary>负载按"几个核心"说：单核百分比 150 → "1.5 核"。</summary>
    public static string Load(uint loadPct) =>
        Loc.Format("cores.load", (loadPct / 100d).ToString(loadPct >= 1000 ? "0" : "0.#", CultureInfo.InvariantCulture));

    /// <summary>用哪些核心：所有核心 / 大核 / 大核和 1 组小核 / 小核。</summary>
    public static string Span(CoreSpan s, Machine m)
    {
        if (s.All)
        {
            return Loc.Get("span.all");
        }
        var prefix = m.Hybrid ? "span.hybrid" : "span.groups";
        if (!s.Main)
        {
            return Loc.Get($"{prefix}.others");
        }
        return s.Others == 0 ? Loc.Get($"{prefix}.main") : Loc.Format($"{prefix}.main_plus", s.Others);
    }

    /// <summary>核心域的位掩码（第 n 位是第 n 个域）换成说法；0 表示不知道。</summary>
    public static string? SpanOfMask(ushort mask, Machine m) => mask == 0
        ? null
        : Span(new CoreSpan(false, (mask & 1) != 0, (uint)BitCount(mask & ~1)), m);

    /// <summary>缓存容量：512 KB、1.25 MB、24 MB。</summary>
    public static string CacheSize(uint bytes)
    {
        const double mb = 1024d * 1024;
        return bytes >= mb
            ? (bytes / mb).ToString("0.##", CultureInfo.InvariantCulture) + " MB"
            : (bytes / 1024d).ToString("0", CultureInfo.InvariantCulture) + " KB";
    }

    private static int BitCount(int v) => System.Numerics.BitOperations.PopCount((uint)v);
}
