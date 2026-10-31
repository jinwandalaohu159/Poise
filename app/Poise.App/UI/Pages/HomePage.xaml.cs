using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Poise.Core;
using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Presentation;
using Poise.App.UI.Theme;
using Poise.App.UI.ViewModels;

namespace Poise.App.UI.Pages;

/// <summary>
/// 主页：只负责把 <see cref="HomeViewModel"/> 的结果画出来，以及转发用户操作。主窗口被挡住或最小化时
/// （托盘面板开着时状态照样每秒更新）不画，重新显示时画一次最新的。
/// 页面留在导航缓存里（从别的页返回不重建），显示期间（<see cref="OnNavigatedTo"/> 到 <see cref="OnNavigatedFrom"/>）才订阅更新。
/// </summary>
public sealed partial class HomePage : Page
{
    private readonly HomeViewModel _vm = AppHost.Current.Home;
    private readonly NumberRoller _freeable;
    private readonly NumberRoller _optimized;
    private bool _rendering;
    /// <summary>主窗口看不见期间有过更新，重新显示时要补画。</summary>
    private bool _stale;
    /// <summary>"游戏时使用"下拉框当前列出的选项对应的机型极限模式（变化时重建选项）。</summary>
    private string? _gameOptionsVendor = "";

    public HomePage()
    {
        InitializeComponent();
        _freeable = new NumberRoller(ShowFreeable, minStep: 1024 * 1024);
        _optimized = new NumberRoller(v => OptimizedValue.Text = v.ToString("0"));

        CleanButton.Label = Loc.Get("home.clean");
        CleanButton.BusyLabel = Loc.Get("home.cleaning");
        CleanButton.Glyph = "\uE74D";
        CleanButton.IsAccent = true;
        CleanButton.Action = () => RunAsync(_vm.CleanAsync);
        OptimizeButton.Label = Loc.Get("home.optimize");
        OptimizeButton.BusyLabel = Loc.Get("home.optimizing");
        OptimizeButton.Glyph = "\uE945";
        OptimizeButton.Action = () => RunAsync(_vm.OptimizeAsync);
        MemoryBar.SetNeutral();
        Modes.ModeChosen += mode => _ = _vm.SetModeAsync(mode);
        Fan.BoostChosen += percent => _ = _vm.SetFanBoostAsync(percent);
        Trend.DetailsRequested += () => Open(typeof(DiagnosePage));
    }

    protected override async void OnNavigatedTo(NavigationEventArgs e)
    {
        _vm.Updated += OnUpdated;
        AppHost.Current.Presence.MainVisibilityChanged += OnMainVisibilityChanged;
        _vm.OptimizeUpdated += RenderOptimize;
        _vm.OpenOptimizations += OnOpenOptimizations;
        if (_vm.Ready)
        {
            Render(_vm.Current);
        }
        await _vm.LoadOptimizeSummaryAsync();
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        _vm.Updated -= OnUpdated;
        AppHost.Current.Presence.MainVisibilityChanged -= OnMainVisibilityChanged;
        _vm.OptimizeUpdated -= RenderOptimize;
        _vm.OpenOptimizations -= OnOpenOptimizations;
    }

    private void OnOpenOptimizations() => Open(typeof(OptimizePage));

    private void OnUpdated(HomeView v)
    {
        if (AppHost.Current.Presence.MainVisible)
        {
            Render(v);
        }
        else
        {
            _stale = true;
        }
    }

    private void OnMainVisibilityChanged()
    {
        if (_stale && AppHost.Current.Presence.MainVisible && _vm.Ready)
        {
            Render(_vm.Current);
        }
    }

    private void Render(HomeView v)
    {
        _stale = false;
        _rendering = true;
        Metrics.Show(v);
        ShowNotice(v);

        Modes.Show(v.Selected, v.Effective, v.InUse, v.VendorMode);
        ShowGameMode(v.GameMode, v.VendorMode);
        Fan.Visibility = v.Fan is null ? Visibility.Collapsed : Visibility.Visible;
        if (v.Fan is { } fan)
        {
            Fan.Show(fan);
        }

        _freeable.RollTo(v.Cleanup.FreeableBytes);
        MemoryBar.Set(v.Cleanup.MemoryFraction, v.Cleanup.FreeableFraction);
        CleanupDetail.Text = v.Cleanup.Detail;

        Modes.IsEnabled = GameModeBox.IsEnabled = Fan.IsEnabled = CleanButton.IsEnabled = OptimizeButton.IsEnabled = v.Connected;
        _rendering = false;
    }

    /// <summary>少见的提醒（见 <see cref="HomeNotice"/>）；没有时收起。</summary>
    private void ShowNotice(HomeView v)
    {
        if (v.Notice is { } notice)
        {
            NoticeBar.Severity = notice.Level == Level.Notice ? InfoBarSeverity.Warning : InfoBarSeverity.Informational;
            NoticeBar.Title = notice.Title;
            NoticeBar.Message = notice.Message;
        }
        SnoozeButton.Visibility = v.Notice?.CanSnooze == true ? Visibility.Visible : Visibility.Collapsed;
        NoticeBar.IsOpen = v.Notice is not null;
    }

    private void ShowGameMode(GameMode mode, string? vendor)
    {
        if (vendor != _gameOptionsVendor)
        {
            _gameOptionsVendor = vendor;
            GameModeBox.ItemsSource = ModeText.GameOptions(vendor);
        }
        var shown = ModeText.ShownGameOption((IReadOnlyList<GameModeOption>)GameModeBox.ItemsSource, mode);
        if (!ReferenceEquals(GameModeBox.SelectedItem, shown))
        {
            GameModeBox.SelectedItem = shown;
        }
    }

    private void OnGameModeChanged(object sender, SelectionChangedEventArgs e)
    {
        if (!_rendering && GameModeBox.SelectedItem is GameModeOption option)
        {
            _ = _vm.SetGameModeAsync(option.Mode);
        }
    }

    /// <summary>数字和单位分别显示。</summary>
    private void ShowFreeable(double bytes) =>
        (FreeableValue.Text, FreeableUnit.Text) = Format.BytesParts((ulong)Math.Max(0, bytes));

    private void RenderOptimize(OptimizeSummary s)
    {
        _optimized.RollTo(s.Optimized);
        OptimizedUnit.Text = $"/ {s.Total}";
        OptimizeBar.Fraction = s.Fraction;
        OptimizeDetail.Text = s.Detail;
    }

    private void OnSnoozeRestart(object sender, RoutedEventArgs e) => _ = _vm.SnoozeRestartAsync();

    private async Task RunAsync(Func<Task<Feedback>> action) => Result.Show(await action());

    private void Open(Type page) => Frame.DrillInto(page);

    private void OnOpenCleanup(object sender, RoutedEventArgs e) => Open(typeof(CleanupPage));

    private void OnOpenOptimize(object sender, RoutedEventArgs e) => Open(typeof(OptimizePage));

    private void OnOpenApps(object sender, RoutedEventArgs e) => Open(typeof(AppsPage));

    private void OnOpenDiagnose(object sender, RoutedEventArgs e) => Open(typeof(DiagnosePage));

    private void OnOpenCores(object sender, RoutedEventArgs e) => Open(typeof(CoresPage));

    private void OnOpenActivity(object sender, RoutedEventArgs e) => Open(typeof(ActivityPage));
}
