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

/// <summary>主页：只负责把 <see cref="HomeViewModel"/> 的结果画出来，以及转发用户操作。</summary>
public sealed partial class HomePage : Page
{
    private readonly HomeViewModel _vm = new(AppHost.Current);
    private readonly NumberRoller _freeable;
    private readonly NumberRoller _optimized;
    private bool _rendering;
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

        _vm.Updated += Render;
        _vm.OptimizeUpdated += RenderOptimize;
        _vm.OpenOptimizations += () => Open(typeof(OptimizePage));
        if (_vm.Ready)
        {
            Render(_vm.Current);
        }
        Loaded += async (_, _) => await _vm.LoadOptimizeSummaryAsync();
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        _vm.Updated -= Render;
        _vm.OptimizeUpdated -= RenderOptimize;
        _vm.Dispose();
    }

    private void Render(HomeView v)
    {
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
        MemoryBar.Fraction = v.Cleanup.MemoryFraction;
        MemoryBar.SecondaryFraction = v.Cleanup.FreeableFraction;
        CleanupDetail.Text = v.Cleanup.Detail;

        Modes.IsEnabled = GameModeBox.IsEnabled = Fan.IsEnabled = CleanButton.IsEnabled = OptimizeButton.IsEnabled = v.Connected;
        _rendering = false;
    }

    /// <summary>少见的提醒：需要重启（可稍后提醒）优先，其次是观察模式。都没有时收起。</summary>
    private void ShowNotice(HomeView v)
    {
        if (v.CanSnoozeRestart)
        {
            NoticeBar.Severity = InfoBarSeverity.Warning;
            NoticeBar.Title = Loc.Get("status.restart");
            NoticeBar.Message = Loc.Get("status.restart.desc");
        }
        else if (v.DryRun)
        {
            NoticeBar.Severity = InfoBarSeverity.Informational;
            NoticeBar.Title = Loc.Get("status.observe");
            NoticeBar.Message = Loc.Get("status.observe.desc");
        }
        SnoozeButton.Visibility = v.CanSnoozeRestart ? Visibility.Visible : Visibility.Collapsed;
        NoticeBar.IsOpen = v.CanSnoozeRestart || v.DryRun;
    }

    private void ShowGameMode(GameMode mode, string? vendor)
    {
        if (vendor != _gameOptionsVendor)
        {
            _gameOptionsVendor = vendor;
            GameModeBox.ItemsSource = ModeText.GameOptions(vendor);
        }
        var options = (IReadOnlyList<GameModeOption>)GameModeBox.ItemsSource;
        // 机型不支持极限模式时，服务按全力处理；下拉框也显示全力。
        var shown = options.FirstOrDefault(o => o.Mode == mode) ?? options.First(o => o.Mode == GameMode.Full);
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

    /// <summary>"1.1 GB" 拆成数字和单位分别显示。</summary>
    private void ShowFreeable(double bytes)
    {
        var text = Format.Bytes((ulong)Math.Max(0, bytes));
        var space = text.LastIndexOf(' ');
        FreeableValue.Text = text[..space];
        FreeableUnit.Text = text[(space + 1)..];
    }

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
