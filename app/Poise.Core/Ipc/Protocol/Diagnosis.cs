// 走势图与卡顿诊断。
using System.Text.Json;

namespace Poise.Core.Ipc;

/// <summary>
/// 走势上的一个点（SecsAgo 越大越早）。PerformancePct、EfficiencyPct 是大核、小核的平均占用，
/// 不是大小核处理器时为 null。
/// </summary>
public sealed record HistoryPoint(float SecsAgo, float CpuPct, float? PerformancePct, float? EfficiencyPct, float MemPct, float? CpuTemp);

/// <summary>最近几分钟的走势：横轴秒数与从早到晚的点（比横轴多几秒）。</summary>
public sealed record Trend(uint WindowSecs, List<HistoryPoint> Points);

/// <summary>
/// 卡顿诊断：结论（cause.kind：thermal / background_cpu / memory_pressure / leaks / foreground_heavy / unclear）与依据。
/// </summary>
public sealed record Diagnosis(
    JsonElement Cause,
    float CpuPct,
    float MemUsedPct,
    float? CpuTemp,
    bool Throttling,
    List<JsonElement> TopCpu,
    List<JsonElement> TopMemory,
    bool SmartSchedule)
{
    public string CauseKind => Text("kind") ?? "unclear";

    public string? CauseName => Text("name");

    /// <summary>后台程序的路径（用于"设为低优先级"）。</summary>
    public string? CausePath => Text("path");

    /// <summary>该程序占整机 CPU 的百分比。</summary>
    public float? CauseShare => Cause.TryGetProperty("share_pct", out var v) && v.ValueKind == JsonValueKind.Number ? v.GetSingle() : null;

    public int? CauseCount => Cause.TryGetProperty("count", out var v) && v.ValueKind == JsonValueKind.Number ? v.GetInt32() : null;

    /// <summary>占用 CPU 最多的程序：名称与占整机的百分比。</summary>
    public IEnumerable<(string Name, double Pct)> CpuRanking =>
        TopCpu.Select(e => (e[0].GetString() ?? "", e[1].GetDouble()));

    /// <summary>占用内存最多的程序：名称与实际占用的物理内存。</summary>
    public IEnumerable<(string Name, ulong Bytes)> MemoryRanking =>
        TopMemory.Select(e => (e[0].GetString() ?? "", e[1].GetUInt64()));

    private string? Text(string name) =>
        Cause.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
}
