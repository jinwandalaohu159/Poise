using System.Globalization;
using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>
/// 日志页的一行：时刻（不是今天的另有日期 Day）、类别图标、发生了什么、补充说明（负载、原因等），需要注意的用橙色。
/// </summary>
public sealed record EventRow(
    ulong Id,
    EventCategory Category,
    string? Day,
    string Time,
    string Glyph,
    string Title,
    string? Detail,
    TagTone Tone,
    string? IconPath);

/// <summary>日志页的筛选项：全部，或某一类。</summary>
public sealed record EventFilter(EventCategory? Category, string Title);

/// <summary>把日志事件写成当前语言的文字（服务只给结构化的数据）。</summary>
public static class EventText
{
    public static IReadOnlyList<EventFilter> Filters() =>
    [
        new(null, Loc.Get("log.filter.all")),
        new(EventCategory.Schedule, Loc.Get("log.category.schedule")),
        new(EventCategory.Mode, Loc.Get("log.category.mode")),
        new(EventCategory.Power, Loc.Get("log.category.power")),
        new(EventCategory.Cleanup, Loc.Get("log.category.cleanup")),
        new(EventCategory.System, Loc.Get("log.category.system")),
    ];

    public static EventRow Row(EventEntry e, Machine machine, DateTimeOffset now)
    {
        var (title, detail, tone, path) = Describe(e.Event, machine);
        var local = DateTimeOffset.FromUnixTimeMilliseconds((long)e.AtUnixMs).ToLocalTime();
        // 今天的只显示时刻（14:02:31），更早的另起一行写日期：时刻一列宽度不变，各行对齐。
        var day = local.Date == now.ToLocalTime().Date ? null : Loc.Format("time.date", local.Month, local.Day);
        return new EventRow(e.Id, e.Category, day, local.ToString("HH:mm:ss", CultureInfo.InvariantCulture), Glyph(e.Category), title, detail, tone, path);
    }

    private static string Glyph(EventCategory c) => c switch
    {
        EventCategory.Schedule => "\uE9F5",
        EventCategory.Mode => "\uE945",
        EventCategory.Power => "\uE7E8",
        EventCategory.Cleanup => "\uE74D",
        _ => "\uE713",
    };

    private static (string Title, string? Detail, TagTone Tone, string? Path) Describe(PoiseEvent ev, Machine m) => ev switch
    {
        DemandEvent d => (Loc.Get(d.Level switch
        {
            Pressure.High => "log.demand.high",
            Pressure.Normal => "log.demand.normal",
            _ => "log.demand.none",
        }), null, TagTone.None, null),
        ProgramEvent p => (Loc.Format("log.program", p.Program.Name, What(p, m)), ProgramDetail(p), TagTone.None, NonEmpty(p.Program.Path)),
        MovedAsideEvent { Count: 0 } => (Loc.Get("log.moved_back"), null, TagTone.None, null),
        MovedAsideEvent a => (Loc.Format(m.Hybrid ? "log.moved_aside" : "log.moved_aside.groups", a.Count), null, TagTone.None, null),
        AccessDeniedEvent a => (Loc.Format("log.access_denied", a.Program.Name), Loc.Get("log.access_denied.desc"), TagTone.Notice, NonEmpty(a.Program.Path)),
        RecoveredEvent r => (Loc.Format("log.recovered", r.Restored), Loc.Format("log.recovered.desc", r.Left), TagTone.Notice, null),
        ModeChangedEvent c => (Loc.Format("log.mode", ModeText.Title(c.Mode, m.Vendor)), c.OnBattery ? Loc.Get("log.on_battery") : null, TagTone.None, null),
        ModeFailedEvent f => (Loc.Format("log.mode_failed", ModeText.Title(f.Mode, m.Vendor)), null, TagTone.Notice, null),
        ThrottledEvent t => (Loc.Get("log.throttled"), t.LimitPct is { } l ? Loc.Format("log.throttled.limit", Format.Percent(l)) : null, TagTone.Notice, null),
        UnthrottledEvent => (Loc.Get("log.unthrottled"), null, TagTone.None, null),
        PlanSwitchedEvent => (Loc.Get("log.plan_switched"), null, TagTone.None, null),
        SettingRestoredEvent s => (Loc.Format("log.setting_restored", Setting(s), Value(s.Found), Value(s.Expected)), Loc.Get(s.Ac ? "log.ac" : "log.dc"), TagTone.Notice, null),
        OptimizationReappliedEvent o => (Loc.Format("log.reapplied", Optimization(o.Id)), null, TagTone.None, null),
        EfficiencySliderEvent => (Loc.Get("log.efficiency_slider"), Loc.Get("log.efficiency_slider.desc"), TagTone.Notice, null),
        OptimizationsAppliedEvent a => (Loc.Format("log.optimized", a.Ids.Count), Optimizations(a.Ids), TagTone.None, null),
        OptimizationsRevertedEvent r => (Loc.Format("log.reverted", r.Ids.Count), Optimizations(r.Ids), TagTone.None, null),
        CleanedEvent c => (Cleaned(c), c.Failed > 0 ? Loc.Format("log.cleaned.failed", c.Failed) : null, TagTone.None, null),
        StartedEvent s => (Loc.Format("log.started", s.Version), s.ObserveOnly ? Loc.Get("log.observe_only") : null, TagTone.None, null),
        ResumedEvent => (Loc.Get("log.resumed"), null, TagTone.None, null),
        PausedEvent => (Loc.Get("log.paused"), null, TagTone.None, null),
        PreferenceSetEvent p => (Loc.Format("log.preference", p.Name, Loc.Get(Preference(p.Preference))), null, TagTone.None, NonEmpty(p.Path)),
        SmartScheduleEvent s => (Loc.Get(s.Enabled ? "log.smart_on" : "log.smart_off"), null, TagTone.None, null),
        _ => (Loc.Get("log.other"), null, TagTone.None, null),
    };

    /// <summary>对程序做了什么。</summary>
    private static string What(ProgramEvent p, Machine m) => p.To switch
    {
        Verdict.Playing => Loc.Format("log.what.playing", CoreWording.Span(p.Cores, m)),
        Verdict.Focused => Loc.Format("log.what.focused", CoreWording.Span(p.Cores, m)),
        Verdict.Yielded when p.Cause == Ipc.Cause.MarkedLow => Loc.Get("log.what.yielded_marked"),
        Verdict.Yielded => Loc.Get("log.what.yielded"),
        Verdict.Lowered => Loc.Get("log.what.lowered"),
        Verdict.Backseat or Verdict.Aside => Loc.Get(m.Hybrid ? "log.what.aside" : "log.what.aside.groups"),
        _ when p.From == Verdict.Playing => Loc.Get("log.what.not_playing"),
        _ when p.From == Verdict.Focused => Loc.Get("log.what.not_focused"),
        _ => Loc.Get("log.what.restored"),
    };

    /// <summary>负载 1.2 核 · 磁盘 3 MB/s · 在前台。</summary>
    private static string ProgramDetail(ProgramEvent p)
    {
        var parts = new List<string>();
        if (p.LoadPct > 0 || p.DiskMbS == 0)
        {
            parts.Add(Loc.Format("log.detail.load", CoreWording.Load(p.LoadPct)));
        }
        if (p.DiskMbS > 0)
        {
            parts.Add(Loc.Format("log.detail.disk", p.DiskMbS));
        }
        if (p.Cause is { } c)
        {
            parts.Add(Loc.Get(CauseKey(c)));
        }
        return string.Join(" · ", parts);
    }

    private static string CauseKey(Cause c) => c switch
    {
        Ipc.Cause.Foreground => "log.cause.foreground",
        Ipc.Cause.RecentlyUsed => "log.cause.recently_used",
        Ipc.Cause.SelfElevated => "log.cause.self_elevated",
        Ipc.Cause.MarkedLow => "log.cause.marked_low",
        Ipc.Cause.Audio => "log.cause.audio",
        Ipc.Cause.Boosted => "log.cause.boosted",
        Ipc.Cause.Marked => "log.cause.marked",
        Ipc.Cause.NoDemand => "log.cause.no_demand",
        Ipc.Cause.LessDemand => "log.cause.less_demand",
        _ => "log.cause.exited",
    };

    private static string Setting(SettingRestoredEvent s)
    {
        var name = Loc.Get(s.Setting switch
        {
            PowerSettingKind.MinState => "log.setting.min_state",
            PowerSettingKind.MaxState => "log.setting.max_state",
            PowerSettingKind.Epp => "log.setting.epp",
            PowerSettingKind.CoreParkingMin => "log.setting.core_parking",
            PowerSettingKind.BoostMode => "log.setting.boost",
            _ => "log.setting.other",
        });
        return s.Class is { } c ? Loc.Format("log.setting.class", name, c) : name;
    }

    private static string Value(uint? v) => v?.ToString(CultureInfo.InvariantCulture) ?? Loc.Get("log.value.none");

    private static string Optimization(string id)
    {
        var key = $"opt.{id}.title";
        var title = Loc.Get(key);
        return title == key ? id : title;
    }

    private static string Optimizations(IEnumerable<string> ids) => string.Join(Loc.Get("log.list_separator"), ids.Select(Optimization));

    private static string Cleaned(CleanedEvent c)
    {
        var what = Loc.Get(c.Trigger switch
        {
            CleanupTrigger.Auto => "log.cleaned.auto",
            CleanupTrigger.Deferred => "log.cleaned.deferred",
            _ => "log.cleaned.manual",
        });
        return c.FreedBytes > 0
            ? Loc.Format("log.cleaned.freed", what, Format.Bytes(c.FreedBytes), c.Handled)
            : Loc.Format("log.cleaned.count", what, c.Handled);
    }

    private static string Preference(AppPreference p) => p switch
    {
        AppPreference.AlwaysFast => "pref.always_fast",
        AppPreference.Background => "pref.background",
        AppPreference.Game => "pref.game",
        _ => "pref.auto",
    };

    private static string? NonEmpty(string s) => string.IsNullOrEmpty(s) ? null : s;
}
