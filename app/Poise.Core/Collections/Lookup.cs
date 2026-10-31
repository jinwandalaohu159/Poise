namespace Poise.Core.Collections;

public static class Lookup
{
    /// <summary>
    /// 按键建字典，键重复时留第一个（<see cref="Enumerable.ToDictionary{TSource, TKey}(IEnumerable{TSource}, Func{TSource, TKey})"/>
    /// 遇到重复键会抛异常：两个同名的硬件读数、同一进程号出现两次时，每次刷新都会出错）。
    /// </summary>
    public static Dictionary<TKey, T> FirstByKey<T, TKey>(this IEnumerable<T> items, Func<T, TKey> key)
        where TKey : notnull
    {
        var map = new Dictionary<TKey, T>();
        foreach (var item in items)
        {
            map.TryAdd(key(item), item);
        }
        return map;
    }
}
