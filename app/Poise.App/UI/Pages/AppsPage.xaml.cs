using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.Core.Localization;
using Poise.App.UI.ViewModels;

namespace Poise.App.UI.Pages;

/// <summary>程序偏好页。列表与偏好修改都在 <see cref="AppsViewModel"/> 里，这里只管显示哪些分组。</summary>
public sealed partial class AppsPage : Page
{
    public AppsPage()
    {
        InitializeComponent();
        Header.Title = Loc.Get("apps.title");
        Vm.Error += message =>
        {
            ErrorBar.Title = message;
            ErrorBar.IsOpen = true;
        };
        Vm.Running.CollectionChanged += (_, _) => UpdateSections();
        Vm.Configured.CollectionChanged += (_, _) => UpdateSections();
        Loaded += OnLoaded;
    }

    public AppsViewModel Vm { get; } = new(AppHost.Current);

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        await Vm.LoadAsync();
        LoadingRing.IsActive = false;
        LoadingRing.Visibility = Visibility.Collapsed;
        UpdateSections();
    }

    private void UpdateSections()
    {
        RunningSection.Visibility = Vm.Running.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        ConfiguredSection.Visibility = Vm.Configured.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        EmptyText.Visibility = Vm.IsEmpty && !LoadingRing.IsActive ? Visibility.Visible : Visibility.Collapsed;
    }
}
