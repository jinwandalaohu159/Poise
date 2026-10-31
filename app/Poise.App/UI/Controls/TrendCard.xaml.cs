using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.App.UI.Services;
using Poise.Core.Localization;
using Poise.Core.Monitoring;
using Poise.Core.Presentation;

namespace Poise.App.UI.Controls;

/// <summary>
/// "最近 5 分钟"走势卡：图例、走势图、当前的性能核 / 能效核负载。主页与卡顿诊断用的是同一个控件，
/// 数据都来自 <see cref="TrendFeed"/>：显示时登记，隐藏时注销，切换页面时新卡片直接接着画。
/// 登记和注销成对、各只做一次（Loaded 可能连着来两次）；窗口关掉时 Unloaded 不一定来，
/// 下一次收到数据时发现自己已经不在界面上，就自己注销。
/// </summary>
public sealed partial class TrendCard : UserControl
{
    private readonly TrendFeed _feed = AppHost.Current.Trend;
    private bool _attached;

    public TrendCard()
    {
        InitializeComponent();
        Chart.StartText = Loc.Get("trend.axis.start");
        Chart.EndText = Loc.Get("trend.axis.now");
        Loaded += (_, _) => Attach();
        Unloaded += (_, _) => Detach();
    }

    private void Attach()
    {
        if (_attached)
        {
            return;
        }
        _attached = true;
        _feed.Updated += OnUpdated;
        Render();
        _feed.Attach(this);
    }

    private void Detach()
    {
        if (!_attached)
        {
            return;
        }
        _attached = false;
        _feed.Updated -= OnUpdated;
        _feed.Detach(this);
    }

    private void OnUpdated()
    {
        if (!IsLoaded || XamlRoot is null)
        {
            Detach();
            return;
        }
        Render();
    }

    /// <summary>走势图的高度（主页较矮）。</summary>
    public double ChartHeight
    {
        get => Chart.Height;
        set => Chart.Height = value;
    }

    /// <summary>是否显示"详情 ›"（主页显示，点了进入卡顿诊断）。</summary>
    public bool ShowDetails
    {
        get => DetailsLink.Visibility == Visibility.Visible;
        set => DetailsLink.Visibility = value ? Visibility.Visible : Visibility.Collapsed;
    }

    public event Action? DetailsRequested;

    private void Render()
    {
        var series = _feed.Latest;
        var hybrid = series.Hybrid || series.Samples.Count == 0;
        PerformanceLegend.Visibility = EfficiencyLegend.Visibility = hybrid ? Visibility.Visible : Visibility.Collapsed;
        CpuLegend.Visibility = hybrid ? Visibility.Collapsed : Visibility.Visible;
        LoadsText.Text = TrendText.Loads(series.Latest);
        CollectingText.Visibility = series.Collecting ? Visibility.Visible : Visibility.Collapsed;
        Chart.Show(series);
    }

    private void OnDetails(object sender, RoutedEventArgs e) => DetailsRequested?.Invoke();
}
