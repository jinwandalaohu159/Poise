using System.Runtime.InteropServices;

namespace Poise.App.UI.Services;

/// <summary>
/// 界面自己的效率模式：主窗口和托盘面板都不显示时开启（降频、放到能效核、放宽计时器精度），
/// 显示时交还给系统。界面在后台等待退出期间不和正在用的程序抢资源。
/// </summary>
internal static class EfficiencyMode
{
    private const int ProcessPowerThrottling = 4;
    private const uint CurrentVersion = 1;
    private const uint ExecutionSpeed = 0x1;
    private const uint IgnoreTimerResolution = 0x4;

    public static void Set(bool on)
    {
        var mask = ExecutionSpeed | IgnoreTimerResolution;
        var state = new PowerThrottlingState
        {
            Version = CurrentVersion,
            ControlMask = on ? mask : 0,
            StateMask = on ? mask : 0,
        };
        _ = SetProcessInformation(GetCurrentProcess(), ProcessPowerThrottling, ref state, (uint)Marshal.SizeOf<PowerThrottlingState>());
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct PowerThrottlingState
    {
        public uint Version;
        public uint ControlMask;
        public uint StateMask;
    }

    [DllImport("kernel32.dll")]
    private static extern IntPtr GetCurrentProcess();

    [DllImport("kernel32.dll")]
    private static extern bool SetProcessInformation(IntPtr process, int infoClass, ref PowerThrottlingState info, uint size);
}
