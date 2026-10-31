using System.Collections.ObjectModel;
using System.Collections.Specialized;
using System.ComponentModel;

namespace Poise.Core.Collections;

/// <summary>
/// 可以一次换掉全部内容的可观察集合：只发一次"重置"通知。整表重新排序时（点列名换排序方式）
/// 比逐行移动便宜——列表把每次移动都当成一次删除加一次插入处理。
/// </summary>
public sealed class ResettableCollection<T> : ObservableCollection<T>
{
    public void ReplaceAll(IEnumerable<T> items)
    {
        CheckReentrancy();
        Items.Clear();
        foreach (var item in items)
        {
            Items.Add(item);
        }
        OnPropertyChanged(new PropertyChangedEventArgs(nameof(Count)));
        OnPropertyChanged(new PropertyChangedEventArgs("Item[]"));
        OnCollectionChanged(new NotifyCollectionChangedEventArgs(NotifyCollectionChangedAction.Reset));
    }
}
