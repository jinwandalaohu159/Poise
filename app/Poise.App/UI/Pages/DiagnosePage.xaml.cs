using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Poise.Core.Diagnostics;
using Poise.Core.Ipc;
using Poise.Core.Localization;
using Poise.Core.Presentation;
using Poise.App.UI.Theme;
using Poise.App.UI.ViewModels;

namespace Poise.App.UI.Pages;

/// <summary>
/// 卡顿诊断页：结论与依据、可直接执行的建议、最近 5 分钟走势、占用排行。
/// 走势卡自己实时刷新（与主页共用，见 <see cref="Controls.TrendCard"/>）；
/// 结论与排行只在打开页面和点"重新分析"时更新，避免文字不停变化。
/// </summary>
public sealed partial class DiagnosePage : Page
{
    private readonly DiagnoseViewModel _vm = new(AppHost.Current);
    private readonly Spin _spin;
    private IReadOnlyList<Suggestion> _shownSuggestions = [];

    public DiagnosePage()
    {
        InitializeComponent();
        _spin = new Spin(AnalyzeIcon);
        Header.Title = Loc.Get("diagnose.title");
    }

    protected override async void OnNavigatedTo(NavigationEventArgs e) => await AnalyzeAsync();

    private async void OnAnalyze(object sender, RoutedEventArgs e) => await AnalyzeAsync();

    private async Task AnalyzeAsync()
    {
        AnalyzeButton.IsEnabled = false;
        _spin.Start();
        SuggestionResult.Visibility = Visibility.Collapsed;
        var (view, error) = await _vm.AnalyzeAsync();
        await _spin.StopAsync();
        AnalyzeButton.IsEnabled = true;
        if (view is null)
        {
            ConclusionText.Text = error ?? Loc.Get("error.generic");
            EvidenceText.Text = "";
            SmartText.Text = "";
            ShowSuggestions([]);
            return;
        }
        Render(view);
    }

    private void Render(DiagnoseView v)
    {
        ConclusionText.Text = v.Conclusion;
        EvidenceText.Text = v.Evidence;
        VisualStateManager.GoToState(this, v.Level == Level.Good ? "Good" : "Notice", true);
        ShowSuggestions(v.Suggestions);
        SmartText.Text = v.SmartSchedule;
        TopCpuList.ItemsSource = v.TopCpu;
        TopMemoryList.ItemsSource = v.TopMemory;
    }

    /// <summary>建议没变时保留原来的按钮（重新分析不让按钮行闪一下）。</summary>
    private void ShowSuggestions(IReadOnlyList<Suggestion> suggestions)
    {
        if (suggestions.SequenceEqual(_shownSuggestions))
        {
            return;
        }
        _shownSuggestions = suggestions;
        Suggestions.Children.Clear();
        for (var i = 0; i < suggestions.Count; i++)
        {
            var s = suggestions[i];
            var button = new Button
            {
                Content = s.Label,
                Style = (Style)Application.Current.Resources[i == 0 ? "AccentButtonStyle" : "DefaultButtonStyle"],
            };
            button.Click += async (_, _) => await OnSuggestion(button, s);
            Suggestions.Children.Add(button);
        }
    }

    private async Task OnSuggestion(Button button, Suggestion s)
    {
        switch (s.Kind)
        {
            case SuggestionKind.OpenApps:
                Open(typeof(AppsPage));
                return;
            case SuggestionKind.OpenCleanup:
                Open(typeof(CleanupPage));
                return;
            case SuggestionKind.OpenOptimize:
                Open(typeof(OptimizePage));
                return;
        }
        button.IsEnabled = false;
        var error = await _vm.ApplyAsync(s);
        if (error is null)
        {
            button.Content = Loc.Get("diagnose.action.done");
        }
        else
        {
            button.IsEnabled = true;
            SuggestionResult.Text = error;
            SuggestionResult.Visibility = Visibility.Visible;
            AppLog.Warn($"诊断建议未完成：{s.Kind}");
        }
    }

    private void Open(Type page) => Frame.DrillInto(page);
}
