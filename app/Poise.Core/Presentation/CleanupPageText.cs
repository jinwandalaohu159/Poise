using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>状态标签的颜色等级：正常不显示标签。</summary>
public enum TagTone { None, Notice, Info }

/// <summary>清理页的一行（像任务管理器：名称、内存、CPU、状态、处理方式）。</summary>
public sealed record ProcessRowView(
    uint Pid,
    string Name,
    string? Path,
    /// <summary>"23 个进程"；只有一个进程时为空。</summary>
    string ProcessCount,
    ulong MemoryBytes,
    string Memory,
    /// <summary>内存条长度：相对于列表中占用最多的一行（0～1）。</summary>
    double MemoryFraction,
    string Cpu,
    string Status,
    TagTone Tone,
    string Action,
    bool NeedsAttention);

public sealed record CleanupPageView(
    ulong FreeableBytes,
    int NeedsAttention,
    /// <summary>"已用 20.1 GB，共 32.0 GB"。</summary>
    string MemoryText,
    double MemoryUsedFraction,
    /// <summary>已用内存中可释放部分占总内存的比例（在内存条上单独标出）。</summary>
    double FreeableFraction,
    bool RestartAdvised,
    bool AutoClean,
    /// <summary>"上次清理 今天 14:02，释放 1.1 GB"；还没清理过时为 null。</summary>
    string? LastCleanup,
    SinceBootView SinceBoot,
    IReadOnlyList<ProcessRowView> Attention,
    IReadOnlyList<ProcessRowView> Others);

/// <summary>把清理明细换成清理页要显示的内容。</summary>
public static class CleanupPageText
{
    /// <summary>重启后会短暂断音的音频服务。</summary>
    private static readonly HashSet<string> AudioServices = new(StringComparer.OrdinalIgnoreCase) { "Audiosrv", "AudioEndpointBuilder" };

    public static CleanupPageView Build(CleanupOverview o, DateTimeOffset now)
    {
        var maxBytes = o.Processes.Count > 0 ? o.Processes.Max(p => p.MemoryBytes) : 0;
        var rows = o.Processes.Select(p => Row(p, maxBytes)).ToList();
        double total = o.MemTotalBytes;
        return new CleanupPageView(
            o.FreeableBytes,
            o.NeedsAttention,
            Loc.Format("cleanup.memory_value", Format.Bytes(o.MemUsedBytes), Format.Bytes(o.MemTotalBytes)),
            total > 0 ? o.MemUsedBytes / total : 0,
            total > 0 ? Math.Min(o.FreeableBytes, o.MemUsedBytes) / total : 0,
            o.RestartAdvised,
            o.AutoClean,
            o.LastCleanup is null ? null : CleanupText.AutoCleanDetail(o.LastCleanup, now),
            SinceBootText.Build(o.SinceBoot, now),
            rows.Where(r => r.NeedsAttention).ToList(),
            rows.Where(r => !r.NeedsAttention).ToList());
    }

    internal static ProcessRowView Row(ProcessGroup p, ulong maxBytes) => new(
        p.Pid,
        p.Name,
        p.Path,
        p.ProcessCount > 1 ? Loc.Format("cleanup.processes", p.ProcessCount) : "",
        p.MemoryBytes,
        Format.Bytes(p.MemoryBytes),
        maxBytes > 0 ? (double)p.MemoryBytes / maxBytes : 0,
        Format.Share(p.CpuPct),
        Status(p),
        p.Issue == ProcessIssue.None ? TagTone.None : p.Issue == ProcessIssue.Heavy ? TagTone.Info : TagTone.Notice,
        Action(p),
        p.Issue != ProcessIssue.None);

    internal static string Status(ProcessGroup p) => p.Issue switch
    {
        ProcessIssue.Growing when p.GrowthPerHour is { } rate => Loc.Format("issue.growing_rate", Format.Bytes(rate)),
        ProcessIssue.Growing => Loc.Get("issue.growing"),
        ProcessIssue.Accumulated when p.SinceStart is { } c => Accumulated(c),
        ProcessIssue.Accumulated => Loc.Get("issue.accumulated"),
        ProcessIssue.Oversized => Loc.Get("issue.oversized"),
        ProcessIssue.Heavy => Loc.Get("issue.heavy"),
        _ => Loc.Get("cleanup.normal"),
    };

    /// <summary>"句柄比启动时多 4.4 万"。</summary>
    private static string Accumulated(Change c)
    {
        var delta = c.To > c.From ? c.To - c.From : 0;
        return c.Metric switch
        {
            GrowthMetric.Handles => Loc.Format("issue.accumulated.handles", Format.Count(delta)),
            GrowthMetric.Threads => Loc.Format("issue.accumulated.threads", Format.Count(delta)),
            _ => Loc.Format("issue.accumulated.memory", Format.Bytes(delta)),
        };
    }

    /// <summary>需要处理、但没有安全的单独处理办法时，说明重启电脑后恢复。</summary>
    internal static string Action(ProcessGroup p) =>
        p.Issue != ProcessIssue.None && p.Remedy.Kind == "none" ? Loc.Get("remedy.none") : Action(p.Remedy);

    internal static string Action(Remedy r) => r.Kind switch
    {
        "restart_service" when r.Service is { } s && AudioServices.Contains(s) => Loc.Get("remedy.restart_audio"),
        "restart_service" => Loc.Format("remedy.restart_service", r.Service ?? ""),
        "recreate_compositor" => Loc.Get("remedy.recreate_compositor"),
        "restart_app" => Loc.Get("remedy.restart_app"),
        "reopen_manually" => Loc.Get("remedy.reopen"),
        _ => "",
    };
}
