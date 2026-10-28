using System.Runtime.InteropServices;

namespace Poise.App.UI.Windows;

/// <summary>
/// 窗口是否被前台的全屏程序（游戏、无边框窗口、演示）挡住。挡住时界面按"不显示"处理：
/// 停止刷新和动画，否则后面的窗口一直在重绘，会让全屏游戏失去独占的显示路径，帧数不稳。
/// </summary>
internal static class FullscreenCover
{
    private const int QunsBusy = 2;               // 全屏程序（包括无边框全屏）
    private const int QunsRunningD3DFullScreen = 3;
    private const int QunsPresentationMode = 4;
    private const uint MonitorDefaultToNearest = 2;

    public static bool Covers(IntPtr hwnd)
    {
        var foreground = GetForegroundWindow();
        if (foreground == IntPtr.Zero || foreground == hwnd)
        {
            return false;
        }
        if (SHQueryUserNotificationState(out var state) != 0
            || state is not (QunsBusy or QunsRunningD3DFullScreen or QunsPresentationMode))
        {
            return false;
        }
        // 多显示器：只有全屏程序和这个窗口在同一块屏幕上才算挡住。
        return MonitorFromWindow(foreground, MonitorDefaultToNearest) == MonitorFromWindow(hwnd, MonitorDefaultToNearest);
    }

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    private static extern IntPtr MonitorFromWindow(IntPtr hwnd, uint flags);

    [DllImport("shell32.dll")]
    private static extern int SHQueryUserNotificationState(out int state);
}
