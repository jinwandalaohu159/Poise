using System.Collections.ObjectModel;

namespace Poise.Core.Collections;

/// <summary>
/// 把一个可观察集合原地调整成目标列表：保留已有对象，只做必要的删除、插入、移动。
/// 列表的过渡动画因此只作用在真正变化的行上（不会整表重新入场）。
/// </summary>
public static class CollectionSync
{
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
}
