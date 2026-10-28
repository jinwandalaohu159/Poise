using System.Xml.Linq;
using Poise.Core.Settings;

namespace Poise.Core.Tests;

public sealed class AutostartTests
{
    private static readonly XNamespace Task = "http://schemas.microsoft.com/windows/2004/02/mit/task";

    private static string Template => File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "AutostartTask.xml"));

    [Fact]
    public void Task_runs_the_agent_at_logon_of_that_user_only()
    {
        const string sid = "S-1-5-21-1-2-3-1001";
        var xml = XDocument.Parse(Autostart.Fill(Template, sid, @"C:\Program Files\Poise\poise-agent.exe"));

        Assert.Equal(sid, xml.Descendants(Task + "LogonTrigger").Single().Element(Task + "UserId")?.Value);
        var principal = xml.Descendants(Task + "Principal").Single();
        Assert.Equal(sid, principal.Element(Task + "UserId")?.Value);
        Assert.Equal("LeastPrivilege", principal.Element(Task + "RunLevel")?.Value);
        Assert.Equal("\"C:\\Program Files\\Poise\\poise-agent.exe\"", xml.Descendants(Task + "Command").Single().Value);
        Assert.Contains(sid, xml.Descendants(Task + "SecurityDescriptor").Single().Value);
        Assert.DoesNotContain("{", xml.ToString());
    }

    [Fact]
    public void Paths_are_escaped()
    {
        var xml = XDocument.Parse(Autostart.Fill(Template, "S-1-5-21-1", @"D:\R&D\<x>\poise-agent.exe"));

        Assert.Equal("\"D:\\R&D\\<x>\\poise-agent.exe\"", xml.Descendants(Task + "Command").Single().Value);
    }
}

/// <summary>在真实的任务计划程序里走一遍注册、查询、删除（任务没有触发器，不会运行），结束后删除测试文件夹。</summary>
public sealed class ScheduledTasksTests : IDisposable
{
    private const string Folder = "PoiseSelfTest";

    private const string Xml = """
        <Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
          <Settings><Enabled>false</Enabled></Settings>
          <Actions><Exec><Command>cmd.exe</Command></Exec></Actions>
        </Task>
        """;

    [Fact]
    public void Register_query_and_delete_without_admin_rights()
    {
        Assert.False(ScheduledTasks.Exists(Folder, "t"));
        ScheduledTasks.Register(Folder, "t", Xml);
        Assert.True(ScheduledTasks.Exists(Folder, "t"));
        ScheduledTasks.Delete(Folder, "t");
        Assert.False(ScheduledTasks.Exists(Folder, "t"));
        ScheduledTasks.Delete(Folder, "t");
    }

    public void Dispose()
    {
        dynamic service = Activator.CreateInstance(Type.GetTypeFromProgID("Schedule.Service")!)!;
        service.Connect();
        try
        {
            service.GetFolder(@"\").DeleteFolder(Folder, 0);
        }
        catch (FileNotFoundException)
        {
        }
    }
}
