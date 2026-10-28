using Microsoft.UI.Xaml.Controls;

namespace Poise.App.UI.Controls;

internal static class InfoBarExtensions
{
    /// <summary>页面顶部的错误提示：显示一句给用户看的原因。</summary>
    public static void ShowError(this InfoBar bar, string message)
    {
        bar.Title = message;
        bar.IsOpen = true;
    }
}
