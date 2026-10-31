using System.Runtime.InteropServices;

namespace Poise.App.UI.Windows;

/// <summary>
/// 窗口是否被前台的最大化窗口（浏览器、编辑器）整个盖住。盖住时界面按"不显示"处理：看不见的窗口不必
/// 每秒刷新、重画走势，服务那边也不用为它读显卡。只算整个盖住的：Poise 的窗口完全在那块屏幕的工作区里
/// （跨屏、露出一部分时照常刷新）。
/// </summary>
internal static class MaximizedCover
{
    private const uint MonitorDefaultToNearest = 2;

    public static bool Covers(IntPtr hwnd)
    {
        var foreground = GetForegroundWindow();
        if (foreground == IntPtr.Zero || foreground == hwnd || !IsZoomed(foreground))
        {
            return false;
        }
        var monitor = MonitorFromWindow(foreground, MonitorDefaultToNearest);
        if (monitor != MonitorFromWindow(hwnd, MonitorDefaultToNearest))
        {
            return false;
        }
        var info = new MonitorInfo { Size = Marshal.SizeOf<MonitorInfo>() };
        if (!GetMonitorInfo(monitor, ref info) || !GetWindowRect(hwnd, out var window))
        {
            return false;
        }
        var work = info.Work;
        return window.Left >= work.Left && window.Top >= work.Top
            && window.Right <= work.Right && window.Bottom <= work.Bottom;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MonitorInfo
    {
        public int Size;
        public Rect Monitor;
        public Rect Work;
        public uint Flags;
    }

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool IsZoomed(IntPtr hwnd);

    [DllImport("user32.dll")]
    private static extern IntPtr MonitorFromWindow(IntPtr hwnd, uint flags);

    [DllImport("user32.dll", EntryPoint = "GetMonitorInfoW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
}
