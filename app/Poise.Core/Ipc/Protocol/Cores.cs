// 核心与缓存面板：处理器的核心域、各自的缓存与负载，以及 Poise 把哪些程序放在哪里。
namespace Poise.Core.Ipc;

/// <summary>
/// 核心与缓存面板。Partitioned 为 false 时只有一个核心域（只有一块缓存），Poise 不限制核心；
/// Domains 按适合跑重点程序的程度排序，第一个是主域（大核，或缓存最大的 CCD）；
/// Unrestricted 是没有限制核心、负载最高的几个程序（由 Windows 安排，可能在任何核心上）。
/// </summary>
public sealed record CoreMap(bool Partitioned, bool SmartSchedule, Pressure Demand, List<CoreDomainInfo> Domains, List<PlacedProgram> Unrestricted);

/// <summary>
/// 一个核心域：效率等级相同、共用一块缓存的核心。L2Bytes 是每块二级缓存的容量、L2Threads 是每块由几个逻辑处理器共用
/// （读不到时都为 0）；L3Group 相同的核心域共用同一块三级缓存。LoadPct 为空表示还没有读数。
/// </summary>
public sealed record CoreDomainInfo(
    bool Performance,
    uint Cores,
    uint Threads,
    uint L2Bytes,
    uint L2Threads,
    uint L3Bytes,
    uint? L3Group,
    float? LoadPct,
    List<ProcessorLoad> Processors,
    List<PlacedProgram> Programs);

/// <summary>一个逻辑处理器（任务管理器里的 CPU 编号）、所属的物理核心（Core 相同的是同一个核心的两个线程）与占用；读不到时为 null。</summary>
public sealed record ProcessorLoad(uint Number, uint Core, float? LoadPct);

/// <summary>被安排在某个核心域上的程序。LoadPct 是单核百分比（200 = 两个核心跑满），Domains 是它同时用着几个核心域。</summary>
public sealed record PlacedProgram(uint Pid, string Name, string Path, Verdict Verdict, uint LoadPct, uint Domains);

/// <summary>对处理器的需求：none 后台不必让路；normal 正在用的程序在忙；high 有重点程序或发热。</summary>
public enum Pressure { None, Normal, High }

/// <summary>
/// 对一个程序的决定：normal 不调整；playing 按游戏调度；focused 重点程序；yielded 完整让路；lowered 只降优先级；
/// backseat 移到小核（不降优先级）；aside 有重点程序时没在用、移到小核。
/// </summary>
public enum Verdict { Normal, Playing, Focused, Yielded, Lowered, Backseat, Aside }
