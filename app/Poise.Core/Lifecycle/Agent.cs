using System.Diagnostics;
using Poise.Core.Diagnostics;
using Poise.Core.Settings;

namespace Poise.Core.Lifecycle;

/// <summary>
/// 托盘代理（poise-agent.exe）负责感知前台窗口并上报服务。界面启动时如果它没在运行（例如没开开机启动），
/// 就把它拉起来，保证调度一直有准确的前台信息。
/// </summary>
public static class Agent
{
    /// <summary>托盘代理持有的单实例互斥体。</summary>
    private const string InstanceMutex = @"Local\Poise.Agent";

    public static bool IsRunning => Mutex.TryOpenExisting(InstanceMutex, out var m) && Dispose(m);

    public static void EnsureRunning()
    {
        if (IsRunning || !Autostart.IsAvailable)
        {
            return;
        }
        try
        {
            Process.Start(new ProcessStartInfo(Autostart.AgentPath) { UseShellExecute = false })?.Dispose();
        }
        catch (Exception e) when (e is System.ComponentModel.Win32Exception or InvalidOperationException)
        {
            AppLog.Warn($"启动托盘代理失败：{e.Message}");
        }
    }

    private static bool Dispose(Mutex m)
    {
        m.Dispose();
        return true;
    }
}
