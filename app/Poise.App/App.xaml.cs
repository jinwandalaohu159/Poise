using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Poise.Core.Diagnostics;
using Poise.Core.Lifecycle;
using Poise.Core.Updates;
using Poise.App.UI.Windows;

namespace Poise.App;

/// <summary>
/// 界面的生命周期。界面按需启动（托盘代理常驻，点托盘图标或"打开 Poise"时才启动界面），
/// 主窗口和托盘面板都关掉后闲置一段时间自动退出，常驻部分只剩几 MB 的托盘代理。
/// <list type="bullet">
/// <item>无参数：打开主窗口；<c>--flyout</c>：打开托盘面板；<c>--update</c>：打开设置页并检查更新。</item>
/// <item>已经在运行时，再次启动只是把请求转给正在运行的界面。</item>
/// </list>
/// </summary>
public partial class App : Application
{
    /// <summary>都不显示后多久退出。期间再次打开是即时的。</summary>
    private static readonly TimeSpan IdleExit = TimeSpan.FromMinutes(10);

    private SingleInstance? _instance;
    private AppHost? _host;
    private DispatcherQueueTimer? _idleExit;
    private MainWindow? _main;
    private TrayFlyoutWindow? _flyout;

    public App()
    {
        InitializeComponent();
        DispatcherShutdownMode = DispatcherShutdownMode.OnExplicitShutdown;
        AppLog.Init();
        UnhandledException += (_, e) => AppLog.Error("未处理的异常", e.Exception);
        TaskScheduler.UnobservedTaskException += (_, e) =>
        {
            AppLog.Error("未观察的任务异常", e.Exception);
            e.SetObserved();
        };
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var request = RequestFromCommandLine(Environment.GetCommandLineArgs());
        _instance = SingleInstance.TryAcquire(request);
        if (_instance is null)
        {
            Exit();
            return;
        }
        var dispatcher = DispatcherQueue.GetForCurrentThread();
        _host = AppHost.Start(dispatcher);
        _idleExit = dispatcher.CreateTimer();
        _idleExit.Interval = IdleExit;
        _idleExit.IsRepeating = false;
        _idleExit.Tick += (_, _) => QuitIfIdle();
        _host.Presence.AllHidden += () => _idleExit.Start();
        _host.Presence.AnyShown += () => _idleExit.Stop();
        // 安装程序会替换正在运行的 Poise：它启动后界面立即退出，不占用文件。
        _host.Updates.InstallerStarted += () => dispatcher.TryEnqueue(Quit);
        _instance.OnRequest(r => dispatcher.TryEnqueue(() => Handle(r)));
        Agent.EnsureRunning();
        Handle(request);
    }

    /// <summary>
    /// 处理外部请求。请求经由 DispatcherQueue 执行，那里抛出的异常不经过 <see cref="Application.UnhandledException"/>，
    /// 会直接结束进程，所以在这里记录并吞掉。
    /// </summary>
    private void Handle(Activation request)
    {
        try
        {
            switch (request)
            {
                case Activation.Window:
                    ShowMain();
                    break;
                case Activation.Flyout:
                    ToggleFlyout();
                    break;
                case Activation.Exit:
                    Quit();
                    break;
                case Activation.Dismiss:
                    _flyout?.Dismiss();
                    break;
                case Activation.Update:
                    ShowUpdate();
                    break;
            }
        }
        catch (Exception e)
        {
            AppLog.Error($"处理请求 {request} 时出错", e);
        }
    }

    private void ShowMain()
    {
        if (_main is not null)
        {
            _main.Present();
            return;
        }
        _main = new MainWindow(_host!);
        _main.Closed += (_, _) => _main = null;
        _main.Activate();
    }

    /// <summary>打开设置页；还不知道有新版本时检查一次。</summary>
    private void ShowUpdate()
    {
        ShowMain();
        _main!.Navigate(typeof(UI.Pages.SettingsPage));
        var updates = _host!.Updates;
        if (updates.State.Phase is not (UpdatePhase.Available or UpdatePhase.Downloading or UpdatePhase.Installing))
        {
            _ = updates.CheckAsync();
        }
    }

    private static Activation RequestFromCommandLine(string[] args)
    {
        bool Has(string flag) => args.Contains(flag, StringComparer.OrdinalIgnoreCase);
        return Has("--flyout") ? Activation.Flyout
            : Has("--update") ? Activation.Update
            : Activation.Window;
    }

    private void ToggleFlyout()
    {
        if (_flyout is null)
        {
            _flyout = new TrayFlyoutWindow(_host!);
            _flyout.OpenMainRequested += ShowMain;
        }
        _flyout.Toggle();
    }

    /// <summary>闲置到时退出；正在检查或下载更新时等它做完再说。</summary>
    private void QuitIfIdle()
    {
        if (_host?.Updates.State.IsBusy == true)
        {
            _idleExit?.Start();
            return;
        }
        Quit();
    }

    private void Quit()
    {
        AppLog.Info("界面退出");
        try
        {
            _main?.Close();
            _flyout?.Close();
            _host?.Dispose();
            _instance?.Dispose();
        }
        finally
        {
            Exit();
        }
    }
}
