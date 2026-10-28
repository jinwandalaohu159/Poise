using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Poise.Core.Localization;
using Poise.App.UI.ViewModels;

namespace Poise.App.UI.Pages;

/// <summary>设置页。所有设置即改即生效，没有"保存"按钮。</summary>
public sealed partial class SettingsPage : Page
{
    public SettingsPage()
    {
        InitializeComponent();
        Header.Title = Loc.Get("settings.title");
    }

    public SettingsViewModel Vm { get; } = new(AppHost.Current);

    protected override void OnNavigatedFrom(NavigationEventArgs e) => Vm.Dispose();
}
