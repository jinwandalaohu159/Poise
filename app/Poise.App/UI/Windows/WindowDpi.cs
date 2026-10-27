using System.Runtime.InteropServices;
using Microsoft.UI.Xaml;

namespace Poise.App.UI.Windows;

internal static class WindowDpi
{
    /// <summary>窗口所在显示器的缩放比例（1.0 = 100%）。内容还没加载时也能用。</summary>
    public static double Scale(this Window window) =>
        GetDpiForWindow(WinRT.Interop.WindowNative.GetWindowHandle(window)) / 96.0;

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr hwnd);
}
