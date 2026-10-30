// 日志：Poise 做过的每一件值得知道的事，结构化记录；界面按语言写成文字（见 Presentation/EventText）。
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Poise.Core.Ipc;

/// <summary>一次取日志的结果：从早到晚的条目，以及详细日志（poise.log）所在的文件夹。</summary>
public sealed record EventPage(List<EventEntry> Entries, string LogDir);

/// <summary>一条日志。Id 递增：界面记住最后一条，下次只取更新的。</summary>
public sealed record EventEntry(ulong Id, ulong AtUnixMs, EventCategory Category, PoiseEvent Event);

public enum EventCategory { Schedule, Mode, Power, Cleanup, System }

/// <summary>涉及的程序（已经退出的只有进程号作名称，路径为空）。</summary>
public sealed record ProgramRef(uint Pid, string Name, string Path);

/// <summary>程序用到的核心范围：All 不限制；Main 用主域（大核）；Others 用了几个其余的核心域。</summary>
public sealed record CoreSpan(bool All, bool Main, uint Others);

/// <summary>程序不（再）让路、或者恢复的原因。</summary>
public enum Cause { Foreground, RecentlyUsed, SelfElevated, MarkedLow, Audio, Boosted, Marked, NoDemand, LessDemand, Exited }

/// <summary>处理器电源设置的种类。</summary>
public enum PowerSettingKind { MinState, MaxState, Epp, CoreParkingMin, BoostMode }

/// <summary>
/// 一件事。按 type 区分具体种类（见 <see cref="PoiseEventConverter"/>）；不认识的种类（更新的服务）读成这个基类，
/// 界面显示为一条普通记录，不会让整页读不出来。
/// </summary>
[JsonConverter(typeof(PoiseEventConverter))]
public record PoiseEvent;

/// <summary>按 type 读出具体的事件类型。</summary>
public sealed class PoiseEventConverter : JsonConverter<PoiseEvent>
{
    private static readonly Dictionary<string, Type> Kinds = new()
    {
        ["demand"] = typeof(DemandEvent),
        ["program"] = typeof(ProgramEvent),
        ["moved_aside"] = typeof(MovedAsideEvent),
        ["access_denied"] = typeof(AccessDeniedEvent),
        ["recovered"] = typeof(RecoveredEvent),
        ["mode_changed"] = typeof(ModeChangedEvent),
        ["mode_failed"] = typeof(ModeFailedEvent),
        ["throttled"] = typeof(ThrottledEvent),
        ["unthrottled"] = typeof(UnthrottledEvent),
        ["plan_switched"] = typeof(PlanSwitchedEvent),
        ["setting_restored"] = typeof(SettingRestoredEvent),
        ["optimization_reapplied"] = typeof(OptimizationReappliedEvent),
        ["efficiency_slider"] = typeof(EfficiencySliderEvent),
        ["optimizations_applied"] = typeof(OptimizationsAppliedEvent),
        ["optimizations_reverted"] = typeof(OptimizationsRevertedEvent),
        ["cleaned"] = typeof(CleanedEvent),
        ["started"] = typeof(StartedEvent),
        ["resumed"] = typeof(ResumedEvent),
        ["paused"] = typeof(PausedEvent),
        ["preference_set"] = typeof(PreferenceSetEvent),
        ["smart_schedule"] = typeof(SmartScheduleEvent),
    };

    public override PoiseEvent Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        using var doc = JsonDocument.ParseValue(ref reader);
        var root = doc.RootElement;
        var kind = root.TryGetProperty("type", out var t) && t.ValueKind == JsonValueKind.String ? t.GetString() : null;
        return kind is not null && Kinds.TryGetValue(kind, out var type)
            ? (PoiseEvent)(root.Deserialize(type, options) ?? new PoiseEvent())
            : new PoiseEvent();
    }

    public override void Write(Utf8JsonWriter writer, PoiseEvent value, JsonSerializerOptions options) =>
        throw new NotSupportedException("界面只读取日志事件");
}

/// <summary>对处理器的需求变了。</summary>
public sealed record DemandEvent(Pressure Level) : PoiseEvent;

/// <summary>对一个程序的决定变了，或重点程序用的核心变了。LoadPct 是单核百分比。</summary>
public sealed record ProgramEvent(
    ProgramRef Program,
    Verdict From,
    Verdict To,
    CoreSpan Cores,
    uint LoadPct,
    uint DiskMbS,
    Cause? Cause) : PoiseEvent;

/// <summary>有重点程序时没在用的程序移到小核：开始时为个数，全部移回时为 0。</summary>
public sealed record MovedAsideEvent(uint Count) : PoiseEvent;

/// <summary>系统拒绝调整这个进程（受保护的进程、反作弊），之后不再尝试。</summary>
public sealed record AccessDeniedEvent(ProgramRef Program) : PoiseEvent;

/// <summary>上次异常退出时留下 Left 个调整过的进程，已把还在运行的 Restored 个恢复原样。</summary>
public sealed record RecoveredEvent(uint Left, uint Restored) : PoiseEvent;

public sealed record ModeChangedEvent(Mode Mode, bool OnBattery) : PoiseEvent;

public sealed record ModeFailedEvent(Mode Mode) : PoiseEvent;

/// <summary>处理器开始被固件限速；LimitPct 是允许的性能上限。</summary>
public sealed record ThrottledEvent(float? LimitPct) : PoiseEvent;

public sealed record UnthrottledEvent : PoiseEvent;

/// <summary>电源计划被换了，当前模式已重新写入新计划。</summary>
public sealed record PlanSwitchedEvent : PoiseEvent;

/// <summary>Poise 写入的电源设置被别处改掉，已改回。Class 是效率等级（不分等级的为空），Ac 为插电。</summary>
public sealed record SettingRestoredEvent(PowerSettingKind? Setting, byte? Class, bool Ac, uint? Found, uint? Expected) : PoiseEvent;

public sealed record OptimizationReappliedEvent(string Id) : PoiseEvent;

/// <summary>Windows 电源模式处于"最佳能效"。</summary>
public sealed record EfficiencySliderEvent : PoiseEvent;

public sealed record OptimizationsAppliedEvent(List<string> Ids) : PoiseEvent;

public sealed record OptimizationsRevertedEvent(List<string> Ids) : PoiseEvent;

public sealed record CleanedEvent(CleanupTrigger Trigger, ulong FreedBytes, uint Handled, uint Failed) : PoiseEvent;

public sealed record StartedEvent(string Version, bool ObserveOnly) : PoiseEvent;

/// <summary>托盘代理连上，开始调度。</summary>
public sealed record ResumedEvent : PoiseEvent;

/// <summary>托盘代理退出：进程调整、散热与电源计划已全部还原。</summary>
public sealed record PausedEvent : PoiseEvent;

public sealed record PreferenceSetEvent(string Name, string Path, AppPreference Preference) : PoiseEvent;

public sealed record SmartScheduleEvent(bool Enabled) : PoiseEvent;
