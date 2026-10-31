using System.Runtime.InteropServices;

namespace Poise.App.UI.Windows;

/// <summary>
/// 窗口是否被系统隐去（在别的虚拟桌面上，或者被挂起）：看不见，界面按"不显示"处理，停止刷新和动画。
/// </summary>
internal static class WindowCloak
{
    private const int DwmwaCloaked = 14;

    public static bool IsCloaked(IntPtr hwnd) =>
        DwmGetWindowAttribute(hwnd, DwmwaCloaked, out var cloaked, sizeof(int)) == 0 && cloaked != 0;

    [DllImport("dwmapi.dll")]
    private static extern int DwmGetWindowAttribute(IntPtr hwnd, int attribute, out int value, int size);
}
