using System.Runtime.InteropServices;

namespace Poise.App.UI.Windows;

/// <summary>
/// 把前台权限交给其他进程。界面在前台时，托盘代理拿不到前台，它的右键菜单就无法在点别处时收起；
/// 收起托盘面板前先交出权限。
/// </summary>
internal static class ForegroundHandoff
{
    private const uint AnyProcess = uint.MaxValue; // ASFW_ANY

    public static void Allow() => AllowSetForegroundWindow(AnyProcess);

    [DllImport("user32.dll")]
    private static extern bool AllowSetForegroundWindow(uint processId);
}
