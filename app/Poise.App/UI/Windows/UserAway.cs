using System.Runtime.InteropServices;

namespace Poise.App.UI.Windows;

/// <summary>
/// 用户不在电脑前：锁着屏、屏幕保护、切到了别的用户，或者这么久没操作。这时开着的窗口没人看，按"不显示"
/// 处理（停止刷新；服务那边也不为它多读传感器、加快采样）。和服务认定"离开"的时间一致。
/// </summary>
internal static class UserAway
{
    private const int QunsNotPresent = 1;
    private const uint AwayAfterMs = 2 * 60_000;

    public static bool Now()
    {
        if (SHQueryUserNotificationState(out var state) == 0 && state == QunsNotPresent)
        {
            return true;
        }
        var info = new LastInputInfo { Size = (uint)Marshal.SizeOf<LastInputInfo>() };
        // 两个都是 32 位毫秒计数，按回绕相减。
        return GetLastInputInfo(ref info) && unchecked((uint)Environment.TickCount - info.Time) >= AwayAfterMs;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct LastInputInfo
    {
        public uint Size;
        public uint Time;
    }

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetLastInputInfo(ref LastInputInfo info);

    [DllImport("shell32.dll")]
    private static extern int SHQueryUserNotificationState(out int state);
}
