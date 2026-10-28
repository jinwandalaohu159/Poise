using Microsoft.UI.Windowing;
using Windows.Graphics;

namespace Poise.App.UI.Windows;

/// <summary>
/// 托盘面板的窗口外观与位置：无标题栏、置顶、不出现在任务切换里；
/// 贴着任务栏放在主屏工作区的角上（任务栏在顶部或左侧时也贴着那条边）。
/// </summary>
internal static class FlyoutPlacement
{
    /// <summary>与工作区边缘的距离（逻辑像素）。</summary>
    private const int EdgeGap = 12;

    public static void ConfigureChrome(AppWindow window)
    {
        window.IsShownInSwitchers = false;
        if (window.Presenter is OverlappedPresenter p)
        {
            p.IsResizable = false;
            p.IsMaximizable = false;
            p.IsMinimizable = false;
            p.IsAlwaysOnTop = true;
            // 保留边框（Windows 11 的圆角与描边），去掉标题栏。
            p.SetBorderAndTitleBar(hasBorder: true, hasTitleBar: false);
        }
    }

    /// <summary>把内容区调整为 <paramref name="width"/>×<paramref name="height"/>（逻辑像素），放到任务栏旁的角上。</summary>
    public static void PlaceNearTaskbar(AppWindow window, double width, double height, double scale)
    {
        var display = DisplayArea.GetFromWindowId(window.Id, DisplayAreaFallback.Primary);
        var work = display.WorkArea;
        var outer = display.OuterBounds;
        var frameW = window.Size.Width - window.ClientSize.Width;
        var frameH = window.Size.Height - window.ClientSize.Height;
        window.Resize(new SizeInt32((int)(width * scale) + frameW, (int)Math.Ceiling(height * scale) + frameH));
        var w = window.Size.Width;
        var h = window.Size.Height;
        var gap = (int)(EdgeGap * scale);

        var x = work.X + work.Width - w - gap;
        var y = work.Y + work.Height - h - gap;
        if (work.Y > outer.Y)
        {
            y = work.Y + gap; // 任务栏在顶部
        }
        if (work.X > outer.X)
        {
            x = work.X + gap; // 任务栏在左侧
        }
        window.Move(new PointInt32(x, y));
    }
}
