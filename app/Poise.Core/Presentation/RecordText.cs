using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>
/// 使用记录的一张卡片：哪些重点程序（主程序的图标）、什么时候、多久、用过的模式，以及几条要点
/// （用了哪些核心、挪开和让路了多少程序、处理器多忙多热、有没有被限速）。
/// </summary>
public sealed record RecordView(
    string Title,
    string When,
    string Duration,
    string Modes,
    bool Game,
    string? IconPath,
    IReadOnlyList<string> Facts,
    bool Throttled);

public static class RecordText
{
    /// <summary>标题里最多列几个程序。</summary>
    private const int TitlePrograms = 3;

    public static RecordView Build(SessionRecord r, Machine machine, DateTimeOffset now)
    {
        var start = DateTimeOffset.FromUnixTimeMilliseconds((long)r.StartedUnixMs);
        var end = DateTimeOffset.FromUnixTimeMilliseconds((long)r.EndedUnixMs);
        var main = r.Programs.FirstOrDefault();
        var title = r.Programs.Count == 0
            ? Loc.Get("records.untitled")
            : string.Join(" · ", r.Programs.Take(TitlePrograms).Select(p => p.Name));
        return new RecordView(
            title,
            Loc.Format("records.when", Format.When(start, now), end.ToLocalTime().ToString("HH:mm")),
            Format.Duration(end - start),
            string.Join(Loc.Get("log.list_separator"), r.Modes.Select(m => ModeText.Title(m, machine.Vendor))),
            r.Programs.Any(p => p.Game),
            main?.Path,
            Facts(r, main, machine),
            r.ThrottledSecs > 0);
    }

    private static IReadOnlyList<string> Facts(SessionRecord r, RecordProgram? main, Machine machine)
    {
        var facts = new List<string>();
        if (main is not null && CoreWording.SpanOfMask(main.WidestDomains, machine) is { } span)
        {
            facts.Add(Loc.Format("records.cores", main.Name, span, CoreWording.Load(main.PeakLoadPct)));
        }
        if (r.MovedAsidePeak > 0 || r.YieldedPrograms > 0)
        {
            facts.Add(Loc.Format(machine.Hybrid ? "records.moved" : "records.moved.groups", r.MovedAsidePeak, r.YieldedPrograms));
        }
        facts.Add(Loc.Format("records.cpu", Format.Percent(r.CpuPctAvg), Format.Percent(r.CpuPctPeak)));
        if (r.CpuTempAvg is { } avg && r.CpuTempPeak is { } peak)
        {
            facts.Add(Loc.Format("records.temp", Format.Temperature(avg), Format.Temperature(peak)));
        }
        facts.Add(r.ThrottledSecs > 0
            ? Loc.Format("records.throttled", Format.Duration(TimeSpan.FromSeconds(r.ThrottledSecs)))
            : Loc.Get("records.not_throttled"));
        return facts;
    }
}
