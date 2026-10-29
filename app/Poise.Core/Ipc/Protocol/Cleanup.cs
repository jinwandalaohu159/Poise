// 清理页：一键清理的结果、清理概览、与开机时相比。
namespace Poise.Core.Ipc;

public enum CleanupTrigger { Manual, Auto, Deferred }

/// <summary>最近一次真正做了事的清理。</summary>
public sealed record LastCleanup(ulong AtUnixMs, CleanupTrigger Trigger, ulong FreedBytes, int Handled);

/// <summary>清理建议。kind：heavy_app / restart_recommended / manual_pending。</summary>
public sealed record Advisory(string Kind, string? Name, ulong? MemoryBytes, int? Count);

public sealed record CleanupResult(ulong FreedBytes, List<string> Handled, List<string> Failed, int Deferred, List<Advisory> Advisories);

/// <summary>accumulated：句柄、线程或内存比进程稳定运行时多出数倍。</summary>
public enum ProcessIssue { None, Growing, Accumulated, Oversized, Heavy }

public enum GrowthMetric { Handles, Threads, Memory }

/// <summary>某一项从基线到现在的变化（内存为字节，其余为个数）。</summary>
public sealed record Change(GrowthMetric Metric, ulong From, ulong To);

/// <summary>一键清理会怎样处理。kind：none / restart_service / recreate_compositor / restart_app / reopen_manually。</summary>
public sealed record Remedy(string Kind, string? Service);

/// <summary>清理页的一行：同一程序的进程合并为一行。</summary>
public sealed record ProcessGroup(
    uint Pid,
    string Name,
    string? Path,
    int ProcessCount,
    /// <summary>实际占用的物理内存（专用工作集，与任务管理器一致）。</summary>
    ulong MemoryBytes,
    float CpuPct,
    ProcessIssue Issue,
    ulong? GrowthPerHour,
    Change? SinceStart,
    Remedy Remedy,
    bool Disruptive);

/// <summary>整机总量的一项。kernel_pool：内核分页池与非分页池之和。</summary>
public enum TotalItem { Handles, Threads, Processes, Commit, KernelPool, SystemCache }

/// <summary>与开机时相比的一项。Notable：增长明显超出正常使用。</summary>
public sealed record TotalChange(TotalItem Item, ulong AtBoot, ulong Now, bool Notable);

public sealed record SinceBoot(ulong BaselineUptimeMs, ulong BaselineAtUnixMs, ulong UptimeMs, List<TotalChange> Items);

public sealed record CleanupOverview(
    ulong FreeableBytes,
    int NeedsAttention,
    ulong MemUsedBytes,
    ulong MemTotalBytes,
    bool RestartAdvised,
    bool AutoClean,
    LastCleanup? LastCleanup,
    /// <summary>开机后系统还没稳定（约 5 分钟）时为空。</summary>
    SinceBoot? SinceBoot,
    List<ProcessGroup> Processes);
