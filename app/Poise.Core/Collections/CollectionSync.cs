using System.Collections.ObjectModel;

namespace Poise.Core.Collections;

/// <summary>
/// 把一个可观察集合原地调整成目标列表：保留已有对象，只做必要的删除、插入、移动。
/// 列表的过渡动画因此只作用在真正变化的行上（不会整表重新入场）。
/// 整表重新排序（超过 <see cref="ResetAbove"/> 的行要挪动）时，<see cref="ResettableCollection{T}"/>
/// 一次换掉全部内容（仍是原来的对象），不逐行移动。要挪动的行按相对顺序算：顶上插进一行，下面的行
/// 只是位置号都加了一，相对顺序没变，不算换了位置。
/// </summary>
public static class CollectionSync
{
    /// <summary>换了位置的行超过这个比例时整表重置。</summary>
    private const double ResetAbove = 0.25;
    /// <summary>行数少于这么多时总是逐行调整（便宜，还有过渡动画）。</summary>
    private const int ResetMinRows = 8;

    public static void Sync<T, TKey>(ObservableCollection<T> target, IReadOnlyList<T> desired, Func<T, TKey> key)
        where TKey : notnull
    {
        var comparer = EqualityComparer<TKey>.Default;
        var wanted = desired.Select(key).ToHashSet();
        for (var i = target.Count - 1; i >= 0; i--)
        {
            if (!wanted.Contains(key(target[i])))
            {
                target.RemoveAt(i);
            }
        }
        if (target is ResettableCollection<T> resettable && Reordered(target, desired, key) is { } ordered)
        {
            resettable.ReplaceAll(ordered);
            return;
        }
        for (var i = 0; i < desired.Count; i++)
        {
            var k = key(desired[i]);
            if (i < target.Count && comparer.Equals(key(target[i]), k))
            {
                continue;
            }
            var from = -1;
            for (var j = i + 1; j < target.Count; j++)
            {
                if (comparer.Equals(key(target[j]), k))
                {
                    from = j;
                    break;
                }
            }
            if (from >= 0)
            {
                target.Move(from, i);
            }
            else
            {
                target.Insert(i, desired[i]);
            }
        }
    }

    /// <summary>大部分行换了位置时，按目标顺序排好的内容（已有的用原来的对象）；否则为 null。</summary>
    private static List<T>? Reordered<T, TKey>(IReadOnlyList<T> current, IReadOnlyList<T> desired, Func<T, TKey> key)
        where TKey : notnull
    {
        if (desired.Count < ResetMinRows || current.Count == 0)
        {
            return null;
        }
        if (Misplaced(current, desired, key) <= desired.Count * ResetAbove)
        {
            return null;
        }
        var existing = current.ToDictionary(key);
        return desired.Select(d => existing.TryGetValue(key(d), out var e) ? e : d).ToList();
    }

    /// <summary>
    /// 两边都有的行里，要挪动才能排成目标顺序的有几行：总数减去按目标顺序已经排好的最长一串
    /// （最长递增子序列，O(n log n)）。
    /// </summary>
    internal static int Misplaced<T, TKey>(IReadOnlyList<T> current, IReadOnlyList<T> desired, Func<T, TKey> key)
        where TKey : notnull
    {
        var order = new Dictionary<TKey, int>(desired.Count);
        for (var i = 0; i < desired.Count; i++)
        {
            order.TryAdd(key(desired[i]), i);
        }
        // tails[k]：长度为 k+1 的递增串里最小的结尾。
        var tails = new List<int>();
        var common = 0;
        foreach (var item in current)
        {
            if (!order.TryGetValue(key(item), out var at))
            {
                continue;
            }
            common++;
            var slot = tails.BinarySearch(at);
            if (slot < 0)
            {
                slot = ~slot;
            }
            if (slot == tails.Count)
            {
                tails.Add(at);
            }
            else
            {
                tails[slot] = at;
            }
        }
        return common - tails.Count;
    }
}
