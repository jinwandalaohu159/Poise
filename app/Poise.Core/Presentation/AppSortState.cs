using Poise.Core.Settings;

namespace Poise.Core.Presentation;

/// <summary>
/// 程序列表的排序：按哪一列、正序还是倒序。点列名的规则与任务管理器一致：
/// 点另一列时按该列的常用方向（名称 A→Z，CPU、内存从高到低）；再点同一列时反过来。
/// </summary>
public readonly record struct AppSortState(AppSort Column, bool Descending)
{
    public static readonly AppSortState Default = new(AppSort.Name, false);

    /// <summary>点了 <paramref name="column"/> 之后的排序。</summary>
    public AppSortState Click(AppSort column) =>
        column == Column ? this with { Descending = !Descending } : new(column, DescendingFirst(column));

    /// <summary>名称默认 A→Z；占用默认从高到低（先看到占用最多的）。</summary>
    private static bool DescendingFirst(AppSort column) => column != AppSort.Name;
}
