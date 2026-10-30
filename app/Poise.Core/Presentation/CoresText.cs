using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>核心与缓存面板的全部内容。Note 是需要说明的情况（只有一块缓存、智能调度关闭）。</summary>
public sealed record CoresView(
    string Summary,
    string Demand,
    string? Note,
    IReadOnlyList<CacheGroupView> Groups,
    IReadOnlyList<PlacedView> Unrestricted);

/// <summary>共用一块三级缓存的几个核心域（读不到三级缓存时每个域单独一组，Title 为空）。</summary>
public sealed record CacheGroupView(string? Title, IReadOnlyList<DomainView> Domains);

/// <summary>
/// 一个核心域：名称、组成（6 核 12 线程）、二级缓存、负载，各个核心，以及 Poise 放在这里的程序
/// （最多 <see cref="CoresText.MaxPrograms"/> 个，其余合成 More）。
/// </summary>
public sealed record DomainView(
    string Title,
    string Shape,
    string? Cache,
    string Load,
    double LoadFraction,
    bool Performance,
    IReadOnlyList<CoreView> Cores,
    IReadOnlyList<PlacedView> Programs,
    string? More);

/// <summary>一个物理核心和它的逻辑处理器（超线程的两个线程在同一个核心里）。</summary>
public sealed record CoreView(IReadOnlyList<ThreadView> Threads);

/// <summary>一个逻辑处理器：CPU 编号、占用文字与比例（读不到时为空）。</summary>
public sealed record ThreadView(string Label, string Load, double Fraction, bool Measured);

/// <summary>一个程序：名称、路径（取图标）、为什么在这里、负载、跨了几组核心。</summary>
public sealed record PlacedView(uint Pid, string Name, string Path, string Role, TagTone Tone, string Load, string? Span);

public static class CoresText
{
    /// <summary>每个核心域最多列几个程序。</summary>
    public const int MaxPrograms = 6;

    public static CoresView Build(CoreMap map, Machine machine)
    {
        return new CoresView(
            Summary(map, machine),
            Loc.Get(map.Demand switch
            {
                Pressure.High => "cores.demand.high",
                Pressure.Normal => "cores.demand.normal",
                _ => "cores.demand.none",
            }),
            Note(map),
            Groups(map, machine),
            [.. map.Unrestricted.Select(p => Placed(p, machine))]);
    }

    /// <summary>共用同一块三级缓存的核心域放在一组（按第一次出现的顺序；读不到三级缓存的各自一组）。</summary>
    private static IReadOnlyList<CacheGroupView> Groups(CoreMap map, Machine machine)
    {
        var groups = new List<(string? Title, List<DomainView> Domains)>();
        var byL3 = new Dictionary<uint, int>();
        for (var i = 0; i < map.Domains.Count; i++)
        {
            var d = map.Domains[i];
            var view = Domain(d, i, map, machine);
            if (d.L3Group is { } g && byL3.TryGetValue(g, out var at))
            {
                groups[at].Domains.Add(view);
                continue;
            }
            if (d.L3Group is { } first)
            {
                byL3[first] = groups.Count;
            }
            groups.Add((L3Title(d), [view]));
        }
        return [.. groups.Select(g => new CacheGroupView(
            g.Title is not null && g.Domains.Count > 1 ? Loc.Format("cores.l3.shared", g.Title, g.Domains.Count) : g.Title,
            g.Domains))];
    }

    private static string Summary(CoreMap map, Machine machine)
    {
        var cores = map.Domains.Sum(d => (long)d.Cores);
        var threads = map.Domains.Sum(d => (long)d.Threads);
        if (machine.Hybrid)
        {
            var big = map.Domains.Where(d => d.Performance).Sum(d => (long)d.Cores);
            return Loc.Format("cores.summary.hybrid", big, cores - big, threads, map.Domains.Count);
        }
        return Loc.Format("cores.summary", cores, threads, map.Domains.Count);
    }

    private static string? Note(CoreMap map) =>
        !map.SmartSchedule ? Loc.Get("cores.note.off")
        : !map.Partitioned ? Loc.Get("cores.note.single")
        : null;

    private static string? L3Title(CoreDomainInfo d) =>
        d.L3Bytes == 0 || d.L3Group is null ? null : Loc.Format("cores.l3", CoreWording.CacheSize(d.L3Bytes));

    private static DomainView Domain(CoreDomainInfo d, int index, CoreMap map, Machine machine)
    {
        var programs = d.Programs.Select(p => Placed(p, machine)).ToList();
        var more = programs.Count > MaxPrograms ? Loc.Format("cores.more", programs.Count - MaxPrograms) : null;
        return new DomainView(
            Title(d, index, map, machine),
            Loc.Format("cores.shape", d.Cores, d.Threads),
            L2(d),
            d.LoadPct is { } load ? Format.Percent(load) : "—",
            Math.Clamp((d.LoadPct ?? 0) / 100d, 0, 1),
            d.Performance,
            Cores(d),
            [.. programs.Take(MaxPrograms)],
            more);
    }

    /// <summary>大核 / 小核 · 第 2 组；不是大小核的处理器：主核心组 / 核心组 2；只有一个域：全部核心。</summary>
    private static string Title(CoreDomainInfo d, int index, CoreMap map, Machine machine)
    {
        if (map.Domains.Count == 1)
        {
            return Loc.Get("cores.domain.all");
        }
        if (machine.Hybrid)
        {
            var sameKind = map.Domains.Where(x => x.Performance == d.Performance).ToList();
            var name = Loc.Get(d.Performance ? "cores.domain.big" : "cores.domain.small");
            return sameKind.Count == 1 ? name : Loc.Format("cores.domain.numbered", name, sameKind.IndexOf(d) + 1);
        }
        return index == 0 ? Loc.Get("cores.domain.main") : Loc.Format("cores.domain.group", index + 1);
    }

    /// <summary>每核 1.25 MB 二级缓存 / 4 核共用 2 MB 二级缓存；读不到时为空。</summary>
    private static string? L2(CoreDomainInfo d)
    {
        if (d.L2Bytes == 0 || d.Cores == 0)
        {
            return null;
        }
        var threadsPerCore = Math.Max(1, d.Threads / d.Cores);
        var sharing = Math.Max(1, d.L2Threads / threadsPerCore);
        var size = CoreWording.CacheSize(d.L2Bytes);
        return sharing == 1 ? Loc.Format("cores.l2.per_core", size) : Loc.Format("cores.l2.shared", sharing, size);
    }

    /// <summary>按物理核心分组（保持系统报告的顺序）。</summary>
    private static IReadOnlyList<CoreView> Cores(CoreDomainInfo d) =>
        [.. d.Processors
            .GroupBy(p => p.Core)
            .Select(g => new CoreView([.. g.Select(Thread)]))];

    private static ThreadView Thread(ProcessorLoad p) => new(
        Loc.Format("cores.cpu", p.Number),
        p.LoadPct is { } l ? Format.Percent(l) : "—",
        Math.Clamp((p.LoadPct ?? 0) / 100d, 0, 1),
        p.LoadPct is not null);

    private static PlacedView Placed(PlacedProgram p, Machine machine) => new(
        p.Pid,
        p.Name,
        p.Path,
        CoreWording.Role(p.Verdict),
        CoreWording.Tone(p.Verdict),
        CoreWording.Load(p.LoadPct),
        p.Domains > 1 ? Loc.Format("cores.spans", p.Domains) : null);
}
