using System.Collections.ObjectModel;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Xaml;
using Poise.App.UI.Services;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>
/// 核心与缓存面板：页面显示期间（<see cref="Start"/> 到 <see cref="Stop"/>）每 <see cref="LiveInterval"/> 读一次
/// （服务只取已有的结果，不额外采样）。处理器的结构只建一次，之后原地更新负载和程序，界面不重建。
/// </summary>
public sealed partial class CoresViewModel(AppHost host) : ObservableObject
{
    private static readonly TimeSpan LiveInterval = TimeSpan.FromSeconds(2);

    private readonly Poller _poller = new(host, LiveInterval);

    public ObservableCollection<CacheGroupItem> Groups { get; } = [];

    public ObservableCollection<PlacedItem> Unrestricted { get; } = [];

    [ObservableProperty] public partial string Summary { get; private set; } = "";
    [ObservableProperty] public partial string Demand { get; private set; } = "";
    [ObservableProperty, NotifyPropertyChangedFor(nameof(NoteVisibility))]
    public partial string Note { get; private set; } = "";

    public Visibility NoteVisibility => Note.Length > 0 ? Visibility.Visible : Visibility.Collapsed;

    public Visibility UnrestrictedEmptyVisibility => Unrestricted.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

    /// <summary>出错时的提示（显示在页面顶部）。</summary>
    public event Action<string>? Error
    {
        add => _poller.Error += value;
        remove => _poller.Error -= value;
    }

    /// <summary>出错之后又读成功了。</summary>
    public event Action? Recovered
    {
        add => _poller.Recovered += value;
        remove => _poller.Recovered -= value;
    }

    public void Start() => _poller.Start(LoadAsync);

    public void Stop() => _poller.Stop();

    private async Task LoadAsync()
    {
        var map = await host.Api.GetCoresAsync();
        Apply(CoresText.Build(map, Machine.From(host.Status.Latest)));
    }

    private void Apply(CoresView view)
    {
        Summary = view.Summary;
        Demand = view.Demand;
        Note = view.Note ?? "";
        var sameShape = view.Groups.Count == Groups.Count && Groups.Zip(view.Groups).All(p => p.First.SameShape(p.Second));
        if (sameShape)
        {
            for (var i = 0; i < Groups.Count; i++)
            {
                Groups[i].Update(view.Groups[i]);
            }
        }
        else
        {
            Groups.Clear();
            foreach (var g in view.Groups)
            {
                Groups.Add(new CacheGroupItem(g));
            }
        }
        var wasEmpty = Unrestricted.Count == 0;
        PlacedItem.SyncInto(Unrestricted, view.Unrestricted);
        if (wasEmpty != (Unrestricted.Count == 0))
        {
            OnPropertyChanged(nameof(UnrestrictedEmptyVisibility));
        }
    }
}
