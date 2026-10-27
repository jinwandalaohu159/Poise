using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Poise.Core.Localization;
using Poise.App.UI.ViewModels;

namespace Poise.App.UI.Pages;

/// <summary>优化记录页：固定项 + 逐个开关的项，以及"全部还原"。</summary>
public sealed partial class OptimizePage : Page
{
    public OptimizePage()
    {
        InitializeComponent();
        Header.Title = Loc.Get("optimize.title");
        Vm.Error += message =>
        {
            ErrorBar.Title = message;
            ErrorBar.IsOpen = true;
        };
        Loaded += async (_, _) => await Vm.LoadAsync();
    }

    public OptimizeViewModel Vm { get; } = new(AppHost.Current);

    public static Visibility VisibleIf(string? text) =>
        string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;

    private async void OnRevertAll(object sender, RoutedEventArgs e)
    {
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = Loc.Get("common.revert_all"),
            Content = Loc.Get("common.revert_all.confirm"),
            PrimaryButtonText = Loc.Get("common.revert"),
            CloseButtonText = Loc.Get("common.cancel"),
            DefaultButton = ContentDialogButton.Close,
            RequestedTheme = ActualTheme,
        };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            RevertAllButton.IsEnabled = false;
            await Vm.RevertAllAsync();
            RevertAllButton.IsEnabled = true;
        }
    }
}

/// <summary>按行的呈现方式选模板。</summary>
public sealed partial class OptimizationRowTemplateSelector : DataTemplateSelector
{
    public DataTemplate? Static { get; set; }

    public DataTemplate? Toggle { get; set; }

    protected override DataTemplate? SelectTemplateCore(object item) =>
        item is OptimizationRow { IsToggle: true } ? Toggle : Static;

    protected override DataTemplate? SelectTemplateCore(object item, DependencyObject container) => SelectTemplateCore(item);
}
