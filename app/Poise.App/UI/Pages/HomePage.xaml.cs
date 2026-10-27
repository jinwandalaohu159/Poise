using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Animation;
using Microsoft.UI.Xaml.Navigation;
using Poise.Core.Localization;
using Poise.Core.Presentation;
using Poise.App.UI.Theme;
using Poise.App.UI.ViewModels;

namespace Poise.App.UI.Pages;

/// <summary>主页：只负责把 <see cref="HomeViewModel"/> 的结果画出来，以及转发用户操作。</summary>
public sealed partial class HomePage : Page
{
    private static readonly TimeSpan ResultAutoClose = TimeSpan.FromSeconds(8);

    private readonly HomeViewModel _vm = new(AppHost.Current);
    private readonly DispatcherTimer _resultTimer = new() { Interval = ResultAutoClose };
    private bool _rendering;
    private string? _shownTitle;

    public HomePage()
    {
        InitializeComponent();
        CleanButton.Label = Loc.Get("home.clean");
        CleanButton.BusyLabel = Loc.Get("home.cleaning");
        CleanButton.Glyph = "\uE74D";
        CleanButton.IsAccent = true;
        CleanButton.Action = () => RunAsync(_vm.CleanAsync);
        OptimizeButton.Label = Loc.Get("home.optimize");
        OptimizeButton.BusyLabel = Loc.Get("home.optimizing");
        OptimizeButton.Glyph = "\uE945";
        OptimizeButton.Action = () => RunAsync(_vm.OptimizeAsync);
        Modes.ModeChosen += mode => _ = _vm.SetModeAsync(mode);
        Motion.EnableTransitions(HeroText);
        _resultTimer.Tick += (_, _) =>
        {
            _resultTimer.Stop();
            ResultBar.IsOpen = false;
        };
        _vm.Updated += Render;
        _vm.OpenOptimizations += () => Frame.Navigate(typeof(OptimizePage), null, new DrillInNavigationTransitionInfo());
        Render(_vm.Current);
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        _resultTimer.Stop();
        _vm.Updated -= Render;
        _vm.Dispose();
    }

    private void Render(HomeView v)
    {
        _rendering = true;
        Dot.Show(v.Level);
        if (_shownTitle is null)
        {
            StatusTitle.Text = v.Title;
        }
        else if (_shownTitle != v.Title)
        {
            // 状态词变化：上移淡出，新词从下方淡入。
            _ = Motion.SwapAsync(HeroText, () => StatusTitle.Text = v.Title);
        }
        _shownTitle = v.Title;
        StatusDescription.Text = v.Description;
        ObserveBadge.Visibility = v.DryRun ? Visibility.Visible : Visibility.Collapsed;
        SnoozeButton.Visibility = v.CanSnoozeRestart ? Visibility.Visible : Visibility.Collapsed;
        AutoCleanDetail.Text = v.AutoCleanDetail;
        CpuTile.Label = v.Cpu.Label;
        CpuTile.Set(v.Cpu.Value, v.Cpu.Format, v.Cpu.Level, v.Cpu.Tooltip);
        GpuTile.Label = v.Gpu.Label;
        GpuTile.Set(v.Gpu.Value, v.Gpu.Format, v.Gpu.Level, v.Gpu.Tooltip);
        MemoryTile.Label = v.Memory.Label;
        MemoryTile.Set(v.Memory.Value, v.Memory.Format, v.Memory.Level, v.Memory.Tooltip);
        Modes.Show(v.Selected, v.Effective);
        AutoFullToggle.IsOn = v.AutoFullForGames;
        AutoCleanToggle.IsOn = v.AutoClean;
        Modes.IsEnabled = CleanButton.IsEnabled = OptimizeButton.IsEnabled = v.Connected;
        _rendering = false;
    }

    private void OnSnoozeRestart(object sender, RoutedEventArgs e) => _ = _vm.SnoozeRestartAsync();

    private void OnAutoFullToggled(object sender, RoutedEventArgs e)
    {
        if (!_rendering)
        {
            _ = _vm.SetAutoFullForGamesAsync(AutoFullToggle.IsOn);
        }
    }

    private void OnAutoCleanToggled(object sender, RoutedEventArgs e)
    {
        if (!_rendering)
        {
            _ = _vm.SetAutoCleanAsync(AutoCleanToggle.IsOn);
        }
    }

    private async Task RunAsync(Func<Task<Feedback>> action)
    {
        ResultBar.IsOpen = false;
        var feedback = await action();
        ShowFeedback(feedback);
    }

    private void ShowFeedback(Feedback f)
    {
        ResultBar.Severity = f.Kind switch
        {
            FeedbackKind.Success => InfoBarSeverity.Success,
            FeedbackKind.Warning => InfoBarSeverity.Warning,
            FeedbackKind.Error => InfoBarSeverity.Error,
            _ => InfoBarSeverity.Informational,
        };
        ResultBar.Title = f.Message;
        ResultBar.Message = string.Join(Environment.NewLine, f.Details);
        ResultBar.ActionButton = f is { ActionLabel: { } label, Action: { } act }
            ? new HyperlinkButton { Content = label, Command = new CommunityToolkit.Mvvm.Input.RelayCommand(act) }
            : null;
        ResultBar.IsOpen = true;
        _resultTimer.Stop();
        _resultTimer.Start();
    }

    private void OnOpenApps(object sender, RoutedEventArgs e) =>
        Frame.Navigate(typeof(AppsPage), null, new DrillInNavigationTransitionInfo());

    private void OnOpenOptimize(object sender, RoutedEventArgs e) =>
        Frame.Navigate(typeof(OptimizePage), null, new DrillInNavigationTransitionInfo());

    private async void OnDiagnoseOpening(object? sender, object e)
    {
        DiagnoseProgress.IsActive = true;
        DiagnoseProgress.Visibility = Visibility.Visible;
        DiagnoseConclusion.Text = "";
        DiagnoseDetails.Visibility = Visibility.Collapsed;
        var view = await _vm.DiagnoseAsync();
        DiagnoseProgress.IsActive = false;
        DiagnoseProgress.Visibility = Visibility.Collapsed;
        if (view is null)
        {
            DiagnoseConclusion.Text = Loc.Get("status.disconnected.desc");
            return;
        }
        DiagnoseConclusion.Text = view.Conclusion;
        DiagnoseCpu.Text = string.Join(Environment.NewLine, view.TopCpu);
        DiagnoseMemory.Text = string.Join(Environment.NewLine, view.TopMemory);
        DiagnoseDetails.Visibility = Visibility.Visible;
    }
}
