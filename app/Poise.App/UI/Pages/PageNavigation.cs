using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Animation;

namespace Poise.App.UI.Pages;

/// <summary>二级页面统一用系统的"钻入"过渡打开（返回见 PageHeader）。</summary>
internal static class PageNavigation
{
    public static void DrillInto(this Frame frame, Type page) =>
        frame.Navigate(page, null, new DrillInNavigationTransitionInfo());
}
