using System.Security;
using System.Security.Principal;
using Poise.Core.Diagnostics;

namespace Poise.Core.Settings;

/// <summary>
/// 登录后自动启动托盘代理（poise-agent.exe，常驻、占用极小；界面按需启动）。
/// <para>
/// 用任务计划程序的登录触发器，而不是 Run 键：Windows 会把 Run 键里的程序推迟到登录后一两分钟才启动，
/// 登录触发的任务则立即启动。任务定义是安装目录里的 AutostartTask.xml（首次安装时服务的 install 命令也用它），
/// 每个用户一个任务，以普通权限运行，创建和删除都不需要管理员权限。
/// </para>
/// 这里的方法都不抛异常：失败时记日志，返回"未开启"或"没有成功"。
/// </summary>
public static class Autostart
{
    private const string TemplateFile = "AutostartTask.xml";

    /// <summary>任务计划程序里的文件夹，卸载时整个删除。</summary>
    private const string TaskFolder = "Poise";

    /// <summary>托盘代理与界面放在同一目录。</summary>
    public static string AgentPath => Path.Combine(AppContext.BaseDirectory, "poise-agent.exe");

    private static string TemplatePath => Path.Combine(AppContext.BaseDirectory, TemplateFile);

    /// <summary>找不到托盘代理时（例如开发时单独运行界面）不能设置开机启动。</summary>
    public static bool IsAvailable => File.Exists(AgentPath) && File.Exists(TemplatePath);

    /// <summary>与服务的 install 命令（poise-service/src/install.rs）使用同一个名字。</summary>
    private static string TaskName => $"Agent {UserSid}";

    private static string UserSid => WindowsIdentity.GetCurrent().User?.Value ?? "";

    public static bool IsEnabled
    {
        get
        {
            try
            {
                return ScheduledTasks.Exists(TaskFolder, TaskName);
            }
            catch (ScheduledTaskException e)
            {
                AppLog.Warn(e.Message);
                return false;
            }
        }
    }

    /// <summary>开启或关闭开机启动，返回是否成功。在后台线程上执行，不卡界面。</summary>
    public static Task<bool> SetAsync(bool enabled) => Task.Run(() => Set(enabled));

    private static bool Set(bool enabled)
    {
        if (enabled && !IsAvailable)
        {
            return false;
        }
        try
        {
            if (enabled)
            {
                ScheduledTasks.Register(TaskFolder, TaskName, Fill(File.ReadAllText(TemplatePath), UserSid, AgentPath));
            }
            else
            {
                ScheduledTasks.Delete(TaskFolder, TaskName);
            }
            return true;
        }
        catch (Exception e) when (e is ScheduledTaskException or IOException)
        {
            AppLog.Warn($"{(enabled ? "开启" : "关闭")}开机启动失败：{e.Message}");
            return false;
        }
    }

    /// <summary>把用户和托盘代理路径填进任务定义。</summary>
    internal static string Fill(string template, string userSid, string agentPath) =>
        template
            .Replace("{UserSid}", SecurityElement.Escape(userSid))
            .Replace("{AgentPath}", SecurityElement.Escape(agentPath));
}
