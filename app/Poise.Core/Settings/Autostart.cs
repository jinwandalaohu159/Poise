using Microsoft.Win32;

namespace Poise.Core.Settings;

/// <summary>
/// 登录后自动启动托盘代理（poise-agent.exe，常驻、占用极小；界面按需启动）。
/// 写在当前用户的 Run 键，不需要管理员权限。
/// </summary>
public static class Autostart
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "Poise";

    /// <summary>托盘代理与界面放在同一目录。</summary>
    public static string AgentPath => Path.Combine(AppContext.BaseDirectory, "poise-agent.exe");

    /// <summary>找不到托盘代理时（例如开发时单独运行界面）不能设置开机启动。</summary>
    public static bool IsAvailable => File.Exists(AgentPath);

    public static bool IsEnabled
    {
        get
        {
            using var key = Registry.CurrentUser.OpenSubKey(RunKey);
            return key?.GetValue(ValueName) is string;
        }
    }

    public static void Set(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(RunKey);
        if (enabled && IsAvailable)
        {
            key.SetValue(ValueName, $"\"{AgentPath}\"");
        }
        else
        {
            key.DeleteValue(ValueName, throwOnMissingValue: false);
        }
    }
}
