using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Poise.Core.Settings;
using Windows.UI;

namespace Poise.App.UI.Theme;

/// <summary>深浅色：默认跟随系统，也可以固定为浅色或深色。切换时整个窗口交叉淡化过渡。</summary>
public sealed class ThemeManager(UiSettings settings)
{
    public ThemeChoice Choice
    {
        get => settings.Theme;
        set
        {
            settings.Theme = value;
            settings.Save();
            Changed?.Invoke();
        }
    }

    public event Action? Changed;

    public ElementTheme ElementTheme => Choice switch
    {
        ThemeChoice.Light => ElementTheme.Light,
        ThemeChoice.Dark => ElementTheme.Dark,
        _ => ElementTheme.Default,
    };

    /// <summary>把主题应用到一个窗口的内容根元素，并同步标题栏按钮颜色。</summary>
    public async void Apply(Window window, bool animate)
    {
        if (window.Content is not FrameworkElement root)
        {
            return;
        }
        if (animate && Motion.Enabled)
        {
            root.OpacityTransition = new ScalarTransition { Duration = Motion.Exit };
            root.Opacity = 0.6;
            await Task.Delay(Motion.Exit);
        }
        root.RequestedTheme = ElementTheme;
        UpdateCaptionButtons(window.AppWindow, root.ActualTheme);
        root.Opacity = 1;
    }

    private static void UpdateCaptionButtons(AppWindow appWindow, ElementTheme actual)
    {
        if (!AppWindowTitleBar.IsCustomizationSupported())
        {
            return;
        }
        var dark = actual == ElementTheme.Dark;
        var fg = dark ? Colors.White : Color.FromArgb(0xE4, 0, 0, 0);
        var bar = appWindow.TitleBar;
        bar.ButtonBackgroundColor = Colors.Transparent;
        bar.ButtonInactiveBackgroundColor = Colors.Transparent;
        bar.ButtonForegroundColor = fg;
        bar.ButtonHoverForegroundColor = fg;
        bar.ButtonHoverBackgroundColor = dark ? Color.FromArgb(0x15, 0xFF, 0xFF, 0xFF) : Color.FromArgb(0x09, 0, 0, 0);
        bar.ButtonPressedBackgroundColor = dark ? Color.FromArgb(0x0B, 0xFF, 0xFF, 0xFF) : Color.FromArgb(0x06, 0, 0, 0);
        bar.ButtonInactiveForegroundColor = dark ? Color.FromArgb(0x87, 0xFF, 0xFF, 0xFF) : Color.FromArgb(0x72, 0, 0, 0);
    }
}
