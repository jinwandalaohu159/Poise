using System.Runtime.InteropServices;

namespace Poise.Core.Settings;

/// <summary>
/// 任务计划程序的最小封装（COM 接口 Schedule.Service）：查询、注册、删除某个文件夹下的任务。
/// 不读注册表里的任务索引：安装程序以管理员身份创建的任务，普通用户没有权限读那里。
/// 出错时抛出 <see cref="ScheduledTaskException"/>。
/// </summary>
internal static class ScheduledTasks
{
    /// <summary>TASK_CREATE_OR_UPDATE。</summary>
    private const int CreateOrUpdate = 6;

    /// <summary>TASK_LOGON_INTERACTIVE_TOKEN：以已登录用户的身份运行，不需要密码。</summary>
    private const int InteractiveToken = 3;

    public static bool Exists(string folder, string name)
    {
        try
        {
            Connect().GetFolder($@"\{folder}").GetTask(name);
            return true;
        }
        catch (Exception e) when (IsNotFound(e))
        {
            return false;
        }
        catch (Exception e) when (IsComFailure(e))
        {
            throw new ScheduledTaskException("查询", e);
        }
    }

    /// <summary>注册或更新任务；文件夹不存在时自动创建。</summary>
    public static void Register(string folder, string name, string xml)
    {
        try
        {
            Connect().GetFolder(@"\").RegisterTask($@"\{folder}\{name}", xml, CreateOrUpdate, null, null, InteractiveToken, null);
        }
        catch (Exception e) when (IsComFailure(e))
        {
            throw new ScheduledTaskException("注册", e);
        }
    }

    public static void Delete(string folder, string name)
    {
        try
        {
            Connect().GetFolder($@"\{folder}").DeleteTask(name, 0);
        }
        catch (Exception e) when (IsNotFound(e))
        {
            // 已经不存在。
        }
        catch (Exception e) when (IsComFailure(e))
        {
            throw new ScheduledTaskException("删除", e);
        }
    }

    private static dynamic Connect()
    {
        var type = Type.GetTypeFromProgID("Schedule.Service", throwOnError: true)!;
        dynamic service = Activator.CreateInstance(type)!;
        service.Connect();
        return service;
    }

    /// <summary>任务或文件夹不存在（HRESULT 0x80070002 / 0x80070003）。</summary>
    private static bool IsNotFound(Exception e) => e is FileNotFoundException or DirectoryNotFoundException;

    private static bool IsComFailure(Exception e) =>
        e is COMException or UnauthorizedAccessException or IOException or ArgumentException or InvalidCastException;
}

public sealed class ScheduledTaskException(string operation, Exception inner)
    : Exception($"{operation}计划任务失败：{inner.Message}", inner);
