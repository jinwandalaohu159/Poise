using System.ComponentModel;
using System.Diagnostics;
using System.Security;
using System.Security.Principal;
using System.Text;
using Microsoft.Win32;
using Poise.Core.Diagnostics;

namespace Poise.Core.Settings;

/// <summary>
/// 登录后自动启动托盘代理（poise-agent.exe，常驻、占用极小；界面按需启动）。
/// <para>
/// 用任务计划程序的登录触发器，而不是 Run 键：Windows 会把 Run 键里的程序推迟到登录后一两分钟才启动，
/// 登录触发的任务则立即启动。任务定义是安装目录里的 AutostartTask.xml（安装脚本也用它），
/// 每个用户一个任务，以普通权限运行，创建和删除都不需要管理员权限。
/// </para>
/// </summary>
public static class Autostart
{
    private const string TemplateFile = "AutostartTask.xml";

    /// <summary>任务计划程序里的文件夹，卸载时整个删除。</summary>
    private const string TaskFolder = "Poise";

    /// <summary>已注册任务的索引（普通用户可读），比启动 schtasks 查询快得多。</summary>
    private const string TaskCacheKey = @"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Schedule\TaskCache\Tree";

    /// <summary>托盘代理与界面放在同一目录。</summary>
    public static string AgentPath => Path.Combine(AppContext.BaseDirectory, "poise-agent.exe");

    private static string TemplatePath => Path.Combine(AppContext.BaseDirectory, TemplateFile);

    /// <summary>找不到托盘代理时（例如开发时单独运行界面）不能设置开机启动。</summary>
    public static bool IsAvailable => File.Exists(AgentPath) && File.Exists(TemplatePath);

    /// <summary>与 install.ps1 使用同一个名字。</summary>
    private static string TaskName => $@"{TaskFolder}\Agent {UserSid}";

    private static string UserSid => WindowsIdentity.GetCurrent().User?.Value ?? "";

    public static bool IsEnabled
    {
        get
        {
            using var key = Registry.LocalMachine.OpenSubKey($@"{TaskCacheKey}\{TaskName}");
            return key is not null;
        }
    }

    /// <summary>开启或关闭开机启动，返回是否成功（失败原因记在日志里）。</summary>
    public static async Task<bool> SetAsync(bool enabled)
    {
        if (enabled == IsEnabled)
        {
            return true;
        }
        if (!enabled)
        {
            return await SchtasksAsync("/Delete", "/TN", TaskName, "/F");
        }
        if (!IsAvailable)
        {
            return false;
        }
        var file = Path.Combine(Path.GetTempPath(), $"poise-autostart-{Environment.ProcessId}.xml");
        try
        {
            var xml = Fill(await File.ReadAllTextAsync(TemplatePath), UserSid, AgentPath);
            // schtasks 只接受 UTF-16 的任务定义文件。
            await File.WriteAllTextAsync(file, xml, Encoding.Unicode);
            return await SchtasksAsync("/Create", "/TN", TaskName, "/XML", file, "/F");
        }
        catch (IOException e)
        {
            AppLog.Warn($"准备开机启动任务失败：{e.Message}");
            return false;
        }
        finally
        {
            File.Delete(file);
        }
    }

    /// <summary>把用户和托盘代理路径填进任务定义。</summary>
    internal static string Fill(string template, string userSid, string agentPath) =>
        template
            .Replace("{UserSid}", SecurityElement.Escape(userSid))
            .Replace("{AgentPath}", SecurityElement.Escape(agentPath));

    private static async Task<bool> SchtasksAsync(params string[] args)
    {
        var info = new ProcessStartInfo("schtasks.exe")
        {
            CreateNoWindow = true,
            UseShellExecute = false,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        foreach (var arg in args)
        {
            info.ArgumentList.Add(arg);
        }
        try
        {
            using var process = Process.Start(info)!;
            var output = process.StandardOutput.ReadToEndAsync();
            var error = process.StandardError.ReadToEndAsync();
            await process.WaitForExitAsync();
            if (process.ExitCode == 0)
            {
                return true;
            }
            AppLog.Warn($"schtasks {args[0]} 失败（{process.ExitCode}）：{(await error).Trim()}{(await output).Trim()}");
            return false;
        }
        catch (Win32Exception e)
        {
            AppLog.Warn($"无法运行 schtasks：{e.Message}");
            return false;
        }
    }
}
