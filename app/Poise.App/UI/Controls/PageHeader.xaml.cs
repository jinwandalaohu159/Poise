using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Animation;
using Poise.Core.Localization;

namespace Poise.App.UI.Controls;

/// <summary>二级页面顶部：返回按钮 + "主页 › 当前页"面包屑。</summary>
public sealed partial class PageHeader : UserControl
{
    public PageHeader()
    {
        InitializeComponent();
        AutomationProperties.SetName(BackButton, Loc.Get("common.back"));
        ToolTipService.SetToolTip(BackButton, Loc.Get("common.back"));
    }

    public string Title
    {
        set => Crumbs.ItemsSource = new[] { Loc.Get("common.home"), value };
    }

    private void OnBack(object sender, RoutedEventArgs e) => GoBack();

    private void OnCrumbClicked(BreadcrumbBar sender, BreadcrumbBarItemClickedEventArgs args)
    {
        if (args.Index == 0)
        {
            GoBack();
        }
    }

    private void GoBack()
    {
        if (FindFrame() is { CanGoBack: true } frame)
        {
            frame.GoBack(new DrillInNavigationTransitionInfo());
        }
    }

    private Frame? FindFrame()
    {
        DependencyObject? node = this;
        while (node is not null and not Frame)
        {
            node = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetParent(node);
        }
        return node as Frame;
    }
}
