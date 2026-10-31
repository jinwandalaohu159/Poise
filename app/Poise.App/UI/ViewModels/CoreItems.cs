using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml;
using Poise.Core.Collections;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

// 核心与缓存面板的各项。结构（几块三级缓存、几组核心、每组几个核心）只在第一次读取和处理器变化时建立，
// 之后每次刷新只原地更新负载和程序，界面不重建、不闪烁。

/// <summary>
/// 共用一块三级缓存的几组核心。大核（不是大小核的处理器全部算大核）各占一整行；小核每组核心少，
/// 在宽窗口里并排（最多两列，见 <see cref="Columns"/>），窄窗口里自动换行。
/// </summary>
public sealed partial class CacheGroupItem : ObservableObject
{
    public CacheGroupItem(CacheGroupView view)
    {
        foreach (var d in view.Domains)
        {
            (d.Performance ? Main : Others).Add(new DomainItem(d));
        }
        Update(view);
    }

    public ObservableCollection<DomainItem> Main { get; } = [];

    public ObservableCollection<DomainItem> Others { get; } = [];

    /// <summary>小核组并排的列数：一组时占满一行，多组时两列。</summary>
    public int Columns => Math.Clamp(Others.Count, 1, 2);

    [ObservableProperty, NotifyPropertyChangedFor(nameof(TitleVisibility))]
    public partial string Title { get; private set; } = "";

    public Visibility TitleVisibility => Title.Length > 0 ? Visibility.Visible : Visibility.Collapsed;

    public Visibility OthersVisibility => Others.Count > 0 ? Visibility.Visible : Visibility.Collapsed;

    internal void Update(CacheGroupView view)
    {
        Title = view.Title ?? "";
        var (main, others) = Split(view);
        for (var i = 0; i < Main.Count; i++)
        {
            Main[i].Update(main[i]);
        }
        for (var i = 0; i < Others.Count; i++)
        {
            Others[i].Update(others[i]);
        }
    }

    /// <summary>结构相同（可以原地更新）。</summary>
    internal bool SameShape(CacheGroupView view)
    {
        var (main, others) = Split(view);
        return Same(Main, main) && Same(Others, others);

        static bool Same(IReadOnlyList<DomainItem> items, IReadOnlyList<DomainView> views) =>
            items.Count == views.Count && items.Zip(views).All(p => p.First.SameShape(p.Second));
    }

    private static (List<DomainView> Main, List<DomainView> Others) Split(CacheGroupView view) =>
        ([.. view.Domains.Where(d => d.Performance)], [.. view.Domains.Where(d => !d.Performance)]);
}

/// <summary>一组核心（核心域）。</summary>
public sealed partial class DomainItem : ObservableObject
{
    public DomainItem(DomainView view)
    {
        Performance = view.Performance;
        foreach (var core in view.Cores)
        {
            Cores.Add(new CoreItem(core, view.Performance));
        }
        Update(view);
    }

    public bool Performance { get; }

    public ObservableCollection<CoreItem> Cores { get; } = [];

    public ObservableCollection<PlacedItem> Programs { get; } = [];

    [ObservableProperty] public partial string Title { get; private set; } = "";
    [ObservableProperty] public partial string Shape { get; private set; } = "";
    [ObservableProperty, NotifyPropertyChangedFor(nameof(CacheVisibility))]
    public partial string Cache { get; private set; } = "";
    [ObservableProperty] public partial string Load { get; private set; } = "";
    [ObservableProperty] public partial double LoadFraction { get; private set; }
    [ObservableProperty, NotifyPropertyChangedFor(nameof(MoreVisibility))]
    public partial string More { get; private set; } = "";

    public Visibility CacheVisibility => Cache.Length > 0 ? Visibility.Visible : Visibility.Collapsed;

    public Visibility MoreVisibility => More.Length > 0 ? Visibility.Visible : Visibility.Collapsed;

    /// <summary>没有程序被限制在这组核心上。</summary>
    public Visibility EmptyVisibility => Programs.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

    internal void Update(DomainView view)
    {
        Title = view.Title;
        Shape = view.Shape;
        Cache = view.Cache ?? "";
        Load = view.Load;
        LoadFraction = view.LoadFraction;
        More = view.More ?? "";
        for (var i = 0; i < Cores.Count; i++)
        {
            Cores[i].Update(view.Cores[i]);
        }
        var wasEmpty = Programs.Count == 0;
        PlacedItem.SyncInto(Programs, view.Programs);
        if (wasEmpty != (Programs.Count == 0))
        {
            OnPropertyChanged(nameof(EmptyVisibility));
        }
    }

    internal bool SameShape(DomainView view) =>
        view.Performance == Performance
        && view.Cores.Count == Cores.Count
        && Cores.Zip(view.Cores).All(p => p.First.Threads.Count == p.Second.Threads.Count);
}

/// <summary>一个物理核心和它的逻辑处理器。</summary>
public sealed class CoreItem
{
    public CoreItem(CoreView view, bool performance)
    {
        foreach (var t in view.Threads)
        {
            Threads.Add(new ThreadItem(t, performance));
        }
    }

    public ObservableCollection<ThreadItem> Threads { get; } = [];

    internal void Update(CoreView view)
    {
        for (var i = 0; i < Threads.Count; i++)
        {
            Threads[i].Update(view.Threads[i]);
        }
    }
}

/// <summary>一个逻辑处理器的小方块：从下往上按占用填色（大核、小核的颜色与走势图一致）。</summary>
public sealed partial class ThreadItem : ObservableObject
{
    public ThreadItem(ThreadView view, bool performance)
    {
        PerformanceVisibility = performance ? Visibility.Visible : Visibility.Collapsed;
        EfficiencyVisibility = performance ? Visibility.Collapsed : Visibility.Visible;
        Update(view);
    }

    public Visibility PerformanceVisibility { get; }

    public Visibility EfficiencyVisibility { get; }

    [ObservableProperty] public partial string Label { get; private set; } = "";
    [ObservableProperty] public partial string Load { get; private set; } = "";
    /// <summary>填色占多少（0～1）。</summary>
    [ObservableProperty] public partial double Fill { get; private set; }

    internal void Update(ThreadView view)
    {
        Label = view.Label;
        Load = view.Load;
        Fill = view.Fraction;
    }
}

/// <summary>面板里的一个程序。</summary>
public sealed partial class PlacedItem : ObservableObject
{
    public PlacedItem(PlacedView view) => Update(view);

    public uint Pid { get; private set; }

    [ObservableProperty] public partial string Name { get; private set; } = "";
    [ObservableProperty] public partial string Path { get; private set; } = "";
    [ObservableProperty] public partial string Role { get; private set; } = "";
    [ObservableProperty] public partial TagTone Tone { get; private set; }
    [ObservableProperty] public partial string Load { get; private set; } = "";
    [ObservableProperty, NotifyPropertyChangedFor(nameof(SpanVisibility))]
    public partial string Span { get; private set; } = "";

    public Visibility SpanVisibility => Span.Length > 0 ? Visibility.Visible : Visibility.Collapsed;

    internal void Update(PlacedView view)
    {
        Pid = view.Pid;
        Name = view.Name;
        Path = view.Path;
        Role = view.Role;
        Tone = view.Tone;
        Load = view.Load;
        Span = view.Span ?? "";
    }

    /// <summary>把列表原地调整成新的内容：同一个程序（按进程号）保留原来的行，只更新文字。</summary>
    internal static void SyncInto(ObservableCollection<PlacedItem> items, IReadOnlyList<PlacedView> views)
    {
        var existing = items.ToDictionary(i => i.Pid);
        var desired = views.Select(v =>
        {
            if (existing.TryGetValue(v.Pid, out var item))
            {
                item.Update(v);
                return item;
            }
            return new PlacedItem(v);
        }).ToList();
        CollectionSync.Sync(items, desired, i => i.Pid);
    }
}
