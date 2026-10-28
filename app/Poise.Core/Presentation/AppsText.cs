using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Settings;

namespace Poise.Core.Presentation;

/// <summary>程序偏好页的文字与排序：与任务管理器"进程"页同样的口径。</summary>
public static class AppsText
{
    /// <summary>占整机 CPU 的百分比，"1.2%"。</summary>
    public static string Cpu(float pct) => $"{pct:0.0}%";

    /// <summary>专用工作集（任务管理器"内存"列）。</summary>
    public static string Memory(ulong bytes) => Format.Bytes(bytes);

    /// <summary>程序行的说明：调度状态，多个进程时加上进程数（"后台 · 能效核 · 优先级较低 · 5 个进程"）；没有运行时为路径。</summary>
    public static string Detail(AppEntry e)
    {
        if (ScheduleText.State(e.Schedule) is not { } state)
        {
            return e.Path;
        }
        return e.Processes.Count > 1 ? $"{state} · {Loc.Format("apps.process_count", e.Processes.Count)}" : state;
    }

    /// <summary>
    /// 按 <paramref name="sort"/> 排序。名称用 <see cref="NameOrder"/>（拼音首字母与英文混排）；
    /// 按占用排序时数值相同的按名称正序，数值不变时顺序也不变。
    /// </summary>
    public static IEnumerable<T> Order<T>(IEnumerable<T> items, AppSortState sort, Func<T, string> name, Func<T, float> cpu, Func<T, ulong> memory) =>
        sort.Column switch
        {
            AppSort.Cpu => By(items, cpu, sort.Descending).ThenBy(name, NameOrder.Instance),
            AppSort.Memory => By(items, memory, sort.Descending).ThenBy(name, NameOrder.Instance),
            _ => sort.Descending ? items.OrderByDescending(name, NameOrder.Instance) : items.OrderBy(name, NameOrder.Instance),
        };

    private static IOrderedEnumerable<T> By<T, TKey>(IEnumerable<T> items, Func<T, TKey> key, bool descending) =>
        descending ? items.OrderByDescending(key) : items.OrderBy(key);

    /// <summary>列名的提示文字，说明再点一次会怎样排序（"按 CPU 从高到低排序"）。</summary>
    public static string SortHint(AppSortState current, AppSort column)
    {
        var next = current.Click(column);
        var key = column switch
        {
            AppSort.Cpu => next.Descending ? "apps.sort.cpu_desc" : "apps.sort.cpu_asc",
            AppSort.Memory => next.Descending ? "apps.sort.memory_desc" : "apps.sort.memory_asc",
            _ => next.Descending ? "apps.sort.name_desc" : "apps.sort.name_asc",
        };
        return Loc.Get(key);
    }
}
