using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>
/// 调度相关的显示：程序偏好页每个程序、每个进程的当前状态（"后台 · 能效核 · 优先级较低"）。
/// 只描述 Poise 做了什么：没调整过的进程显示"未调整"，不把程序自己设的优先级当成 Poise 的调整。
/// </summary>
public static class ScheduleText
{
    /// <summary>程序当前的调度状态；没有运行时为 null。</summary>
    public static string? State(AppSchedule? s)
    {
        if (s is null)
        {
            return null;
        }
        return s.State switch
        {
            ScheduleState.Excluded => Loc.Get("sched.excluded"),
            ScheduleState.SelfManaged => Loc.Get("sched.self_managed"),
            ScheduleState.Off => Loc.Get("sched.off"),
            _ => string.Join(" · ", Parts(s)),
        };
    }

    private static IEnumerable<string> Parts(AppSchedule s)
    {
        yield return Loc.Get(s.Foreground ? "sched.foreground" : "sched.background");
        switch (s.Adjustment)
        {
            case Adjustment.None:
                yield return Loc.Get("sched.unadjusted");
                yield break;
            case Adjustment.Protected:
                if (s.Cores == CoreUse.Performance)
                {
                    yield return Loc.Get("sched.performance_cores");
                }
                yield return Loc.Get("sched.protected");
                yield break;
            case Adjustment.Raised:
                if (s.Cores == CoreUse.Performance)
                {
                    yield return Loc.Get("sched.performance_cores");
                }
                yield return Loc.Get(Priority(s.Priority));
                yield return Loc.Get("sched.protected");
                yield break;
        }
        if (s.Cores is { } cores)
        {
            yield return Loc.Get(cores switch
            {
                CoreUse.Efficiency => "sched.small_cores",
                CoreUse.Performance => "sched.performance_cores",
                _ => "sched.any_cores",
            });
        }
        yield return Loc.Get(Priority(s.Priority));
    }

    private static string Priority(PriorityLevel p) => p switch
    {
        PriorityLevel.Lower => "sched.priority.lower",
        PriorityLevel.Higher => "sched.priority.higher",
        PriorityLevel.High => "sched.priority.high",
        _ => "sched.priority.normal",
    };
}
