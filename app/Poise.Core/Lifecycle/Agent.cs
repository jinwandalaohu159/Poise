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

    /// <summary>托盘代理等着的"界面设置变了"事件（见 poise-agent 的 prefs::watch）。</summary>
    private const string PrefsEvent = @"Local\Poise.Agent.Prefs";

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

    /// <summary>告诉托盘代理界面设置变了（语言、提醒圆点），它立即刷新托盘图标。</summary>
    public static void PrefsChanged()
    {
        if (EventWaitHandle.TryOpenExisting(PrefsEvent, out var changed))
        {
            using (changed)
            {
                changed.Set();
            }
        }
    }

    private static bool Dispose(Mutex m)
    {
        m.Dispose();
        return true;
    }
}
