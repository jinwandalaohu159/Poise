using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

public sealed record DiagnoseView(string Conclusion, IReadOnlyList<string> TopCpu, IReadOnlyList<string> TopMemory);

/// <summary>卡顿诊断结果的文案。</summary>
public static class DiagnoseText
{
    private const int Shown = 3;

    public static DiagnoseView Build(StutterReport r)
    {
        var conclusion = r.CauseKind switch
        {
            "thermal" => Loc.Get("diagnose.thermal"),
            "background_cpu" => Loc.Format("diagnose.background_cpu", r.CauseName),
            "memory_pressure" => Loc.Get("diagnose.memory_pressure"),
            "leaks" => Loc.Format("diagnose.leaks", r.Cause.TryGetProperty("count", out var c) ? c.GetInt32() : 0),
            "foreground_heavy" => Loc.Format("diagnose.foreground_heavy", r.CauseName),
            _ => Loc.Get("diagnose.unclear"),
        };
        // CPU 占比是整机百分比（与任务管理器一致）。
        var cpu = r.TopCpu.Take(Shown).Select(e => $"{e[0].GetString()}  {e[1].GetDouble():0}%").ToList();
        var mem = r.TopMemory.Take(Shown).Select(e => $"{e[0].GetString()}  {Format.Bytes(e[1].GetUInt64())}").ToList();
        return new DiagnoseView(conclusion, cpu, mem);
    }
}
