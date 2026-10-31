using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Poise.App.UI.Controls;
using Poise.Core;
using Poise.Core.Localization;
using Poise.Core.Presentation;
using Poise.App.UI.Theme;
using Poise.App.UI.ViewModels;

namespace Poise.App.UI.Pages;

/// <summary>清理页：顶部摘要与一键清理，下方按程序列出占用与处理方式。</summary>
public sealed partial class CleanupPage : Page
{
    private readonly NumberRoller _freeable;
    private readonly NumberRoller _attention;
    private bool _rendering;

    public CleanupPage()
    {
        InitializeComponent();
        Header.Title = Loc.Get("cleanup.title");
        _freeable = new NumberRoller(ShowFreeable, minStep: 1024 * 1024);
        _attention = new NumberRoller(v => AttentionValue.Text = Math.Round(v).ToString("0"));
        CleanButton.Label = Loc.Get("home.clean");
        CleanButton.BusyLabel = Loc.Get("home.cleaning");
        CleanButton.Glyph = "\uE74D";
        CleanButton.IsAccent = true;
        CleanButton.Action = async () => Result.Show(await Vm.CleanAsync());
        Vm.Error += ErrorBar.ShowError;
        Vm.Attention.CollectionChanged += (_, _) => UpdateSections();
        Vm.Others.CollectionChanged += (_, _) => UpdateSections();
    }

    public CleanupViewModel Vm { get; } = new(AppHost.Current);

    /// <summary>订阅与取消成对：进页面时订阅、开始刷新，离开时取消、停止。</summary>
    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        Vm.Updated += Render;
        Vm.Start();
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        Vm.Updated -= Render;
        Vm.Dispose();
    }

    private void Render(CleanupPageView v)
    {
        _rendering = true;
        ErrorBar.IsOpen = false;
        _freeable.RollTo(v.FreeableBytes);
        _attention.RollTo(v.NeedsAttention);
        MemoryText.Text = $"{Loc.Get("cleanup.memory")}  {v.MemoryText}";
        MemoryBar.Set(v.MemoryUsedFraction, v.FreeableFraction);
        LastCleanText.Text = v.LastCleanup ?? "";
        LastCleanText.Visibility = v.LastCleanup is null ? Visibility.Collapsed : Visibility.Visible;
        AutoCleanToggle.IsOn = v.AutoClean;
        RestartBar.IsOpen = v.RestartAdvised;
        SinceBoot.Show(v.SinceBoot);
        SinceBootSection.Visibility = Visibility.Visible;
        LoadingRing.IsActive = false;
        LoadingRing.Visibility = Visibility.Collapsed;
        _rendering = false;
    }

    private void ShowFreeable(double bytes) =>
        (FreeableValue.Text, FreeableUnit.Text) = Format.BytesParts((ulong)Math.Max(0, bytes));

    private void UpdateSections()
    {
        AttentionSection.Visibility = Vm.Attention.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        OthersSection.Visibility = Vm.Others.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    private void OnAutoCleanToggled(object sender, RoutedEventArgs e)
    {
        if (!_rendering)
        {
            _ = Vm.SetAutoCleanAsync(AutoCleanToggle.IsOn);
        }
    }

    private void OnSnoozeRestart(object sender, RoutedEventArgs e) => _ = Vm.SnoozeRestartAsync();
}
