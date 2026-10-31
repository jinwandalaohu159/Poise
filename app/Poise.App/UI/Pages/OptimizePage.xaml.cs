using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using Poise.App.UI.Controls;
using Poise.Core.Localization;
using Poise.Core.Presentation;
using Poise.App.UI.ViewModels;

namespace Poise.App.UI.Pages;

/// <summary>系统优化页：顶部进度与一键优化；下方是固定项与逐个开关的项。</summary>
public sealed partial class OptimizePage : Page
{
    public OptimizePage()
    {
        InitializeComponent();
        Header.Title = Loc.Get("optimize.title");
        OptimizeButton.Label = Loc.Get("home.optimize");
        OptimizeButton.BusyLabel = Loc.Get("home.optimizing");
        OptimizeButton.Glyph = "\uE945";
        OptimizeButton.IsAccent = true;
        OptimizeButton.Action = async () => Result.Show(await Vm.OptimizeAllAsync());
        Vm.SummaryChanged += ShowSummary;
        Vm.Error += ErrorBar.ShowError;
    }

    public OptimizeViewModel Vm { get; } = new(AppHost.Current);

    protected override async void OnNavigatedTo(NavigationEventArgs e) => await Vm.LoadAsync();

    private void ShowSummary(OptimizeSummary s)
    {
        SummaryText.Text = s.Text;
        SummaryBar.Fraction = s.Fraction;
    }


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
