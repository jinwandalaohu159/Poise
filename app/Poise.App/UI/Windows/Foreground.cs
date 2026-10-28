using System.Runtime.InteropServices;

namespace Poise.App.UI.Windows;

/// <summary>
/// 前台窗口的转交与获取。
/// 界面在前台时，托盘代理拿不到前台，它的右键菜单就无法在点别处时收起：收起托盘面板前先交出权限（<see cref="AllowOthers"/>）。
/// 托盘面板弹出时要真正成为前台窗口（<see cref="Take"/>），否则点别处时收不到失去焦点的通知。
/// </summary>
internal static class Foreground
{
    private const uint AnyProcess = uint.MaxValue; // ASFW_ANY

    public static void AllowOthers() => AllowSetForegroundWindow(AnyProcess);

    /// <summary>把窗口设为前台（托盘代理在用户点击托盘图标时已把前台权限转交给界面）。</summary>
    public static void Take(IntPtr hwnd) => SetForegroundWindow(hwnd);

    [DllImport("user32.dll")]
    private static extern bool AllowSetForegroundWindow(uint processId);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr hwnd);
}
