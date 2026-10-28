using Poise.Core.Ipc;
using Poise.Core.Localization;

namespace Poise.Core.Presentation;

/// <summary>
/// 调度相关的显示：程序偏好页每个程序、每个进程的当前状态（"后台 · 能效核 · 优先级较低"）。
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
        if (s.SmallCores is { } small)
        {
            yield return Loc.Get(small ? "sched.small_cores" : "sched.any_cores");
        }
        yield return Loc.Get(s.Priority switch
        {
            PriorityLevel.Lower => "sched.priority.lower",
            PriorityLevel.Higher => "sched.priority.higher",
            PriorityLevel.High => "sched.priority.high",
            _ => "sched.priority.normal",
        });
    }
}
