using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>诊断后可以直接执行的建议。</summary>
public enum SuggestionKind
{
    /// 把某个程序设为低优先级（<see cref="Suggestion.Path"/>）。
    SetLowPriority,
    OpenApps,
    OpenCleanup,
    /// 打开系统优化页（电源设置、右键菜单扩展）。
    OpenOptimize,
    SwitchToFull,
}

public sealed record Suggestion(SuggestionKind Kind, string Label, string? Path = null);

/// <summary>排行里的一项：名称、数值文字、条形长度（相对第一名，0～1）。</summary>
public sealed record BarItem(string Name, string Value, double Fraction);

public sealed record DiagnoseView(
    string Conclusion,
    /// <summary>结论的依据（带关键数字）。</summary>
    string Evidence,
    /// <summary>没发现明显问题时为 Good，否则为 Notice。</summary>
    Level Level,
    IReadOnlyList<Suggestion> Suggestions,
    IReadOnlyList<BarItem> TopCpu,
    IReadOnlyList<BarItem> TopMemory,
    /// <summary>"智能调度：已开启"（便于开关前后对比）。</summary>
    string SmartSchedule);

/// <summary>卡顿诊断：结论、依据和建议的文案。</summary>
public static class DiagnoseText
{
    private const int Shown = 5;

    public static DiagnoseView Build(Diagnosis d)
    {
        var cpu = $"{d.CpuPct:0}";
        var share = $"{d.CauseShare ?? 0:0}";
        var name = d.CauseName ?? "";
        var (conclusion, evidence, suggestions) = d.CauseKind switch
        {
            "thermal" => (Loc.Get(d.Throttling ? "diagnose.thermal" : "diagnose.thermal_hot"), ThermalEvidence(d),
                new[] { new Suggestion(SuggestionKind.OpenOptimize, Loc.Get("diagnose.action.power")) }),
            "background_cpu" => (Loc.Format("diagnose.background_cpu", name),
                Loc.Format("diagnose.evidence.share", cpu, name, share),
                d.CausePath is { } path
                    ? new[]
                    {
                        new Suggestion(SuggestionKind.SetLowPriority, Loc.Format("diagnose.action.low_priority", name), path),
                        new Suggestion(SuggestionKind.OpenApps, Loc.Get("diagnose.action.apps")),
                    }
                    : [new Suggestion(SuggestionKind.OpenApps, Loc.Get("diagnose.action.apps"))]),
            "memory_pressure" => (Loc.Get("diagnose.memory_pressure"),
                Loc.Format("diagnose.evidence.memory", $"{d.MemUsedPct:0}"),
                new[] { new Suggestion(SuggestionKind.OpenCleanup, Loc.Get("diagnose.action.cleanup")) }),
            "leaks" => (Loc.Format("diagnose.leaks", d.CauseCount ?? 0),
                Loc.Format("diagnose.evidence.leaks", d.CauseCount ?? 0),
                new[] { new Suggestion(SuggestionKind.OpenCleanup, Loc.Get("diagnose.action.cleanup")) }),
            "foreground_heavy" => (Loc.Format("diagnose.foreground_heavy", name),
                Loc.Format("diagnose.evidence.share", cpu, name, share),
                new[] { new Suggestion(SuggestionKind.SwitchToFull, Loc.Get("diagnose.action.full")) }),
            _ => (Loc.Get("diagnose.unclear"),
                Loc.Format("diagnose.evidence.normal", cpu, $"{d.MemUsedPct:0}"),
                new[] { new Suggestion(SuggestionKind.OpenOptimize, Loc.Get("diagnose.action.context_menu")) }),
        };
        var level = d.CauseKind == "unclear" ? Level.Good : Level.Notice;
        return new DiagnoseView(conclusion, evidence, level, suggestions,
            Bars(d.CpuRanking, Format.Share),
            Bars(d.MemoryRanking.Select(m => (m.Name, (double)m.Bytes)), v => Format.Bytes((ulong)v)),
            Loc.Get(d.SmartSchedule ? "diag.smart_on" : "diag.smart_off"));
    }

    private static string ThermalEvidence(Diagnosis d)
    {
        var parts = new List<string>();
        if (d.CpuTemp is { } t)
        {
            parts.Add(Loc.Format("diagnose.evidence.temp", $"{t:0}"));
        }
        if (d.Throttling || parts.Count == 0)
        {
            parts.Add(Loc.Get("diagnose.evidence.throttling"));
        }
        return string.Join(Loc.Language == Loc.Chinese ? "，" : ", ", parts);
    }

    private static List<BarItem> Bars(IEnumerable<(string Name, double Value)> items, Func<double, string> format)
    {
        var list = items.Take(Shown).ToList();
        var max = list.Count > 0 ? list.Max(i => i.Value) : 0;
        return list.Select(i => new BarItem(i.Name, format(i.Value), max > 0 ? i.Value / max : 0)).ToList();
    }
}
