namespace Poise.Core.Lifecycle;

/// <summary>界面收到的外部请求（来自托盘代理，或用户再次启动 Poise）。</summary>
public enum Activation
{
    /// 打开主窗口。
    Window,
    /// 开关托盘面板。
    Flyout,
    /// 退出（托盘菜单"退出"）。
    Exit,
    /// 收起托盘面板并交出前台（托盘代理弹出右键菜单前）。
    Dismiss,
    /// 打开主窗口的设置页并检查更新（托盘代理发现新版本后，用户点了通知或菜单）。
    Update,
}

/// <summary>
/// 每个用户只运行一个界面。后启动的实例把请求转给先启动的实例，然后自己退出。
/// 托盘代理也用同样的事件名唤起界面。
/// </summary>
public sealed class SingleInstance : IDisposable
{
    private const string MutexName = @"Local\Poise.App";

    private static readonly IReadOnlyDictionary<Activation, string> SignalNames = new Dictionary<Activation, string>
    {
        [Activation.Window] = @"Local\Poise.App.Activate",
        [Activation.Flyout] = @"Local\Poise.App.Flyout",
        [Activation.Exit] = @"Local\Poise.App.Exit",
        [Activation.Dismiss] = @"Local\Poise.App.Dismiss",
        [Activation.Update] = @"Local\Poise.App.Update",
    };

    private readonly Mutex _mutex;
    private readonly Dictionary<Activation, EventWaitHandle> _signals;
    private readonly List<RegisteredWaitHandle> _waits = [];

    private SingleInstance(Mutex mutex, Dictionary<Activation, EventWaitHandle> signals)
    {
        _mutex = mutex;
        _signals = signals;
    }

    /// <summary>成为首个实例则返回守卫；否则把 <paramref name="request"/> 转给首个实例并返回 null。</summary>
    public static SingleInstance? TryAcquire(Activation request)
    {
        var mutex = new Mutex(initiallyOwned: true, MutexName, out var created);
        var signals = SignalNames.ToDictionary(
            p => p.Key,
            p => new EventWaitHandle(false, EventResetMode.AutoReset, p.Value));
        if (!created)
        {
            signals[request].Set();
            foreach (var s in signals.Values)
            {
                s.Dispose();
            }
            mutex.Dispose();
            return null;
        }
        return new SingleInstance(mutex, signals);
    }

    /// <summary>收到请求时调用（在线程池线程上，调用方自行切回界面线程）。</summary>
    public void OnRequest(Action<Activation> callback)
    {
        foreach (var (kind, signal) in _signals)
        {
            _waits.Add(ThreadPool.RegisterWaitForSingleObject(
                signal, (_, _) => callback(kind), null, Timeout.Infinite, executeOnlyOnce: false));
        }
    }

    public void Dispose()
    {
        foreach (var w in _waits)
        {
            w.Unregister(null);
        }
        foreach (var s in _signals.Values)
        {
            s.Dispose();
        }
        _mutex.ReleaseMutex();
        _mutex.Dispose();
    }
}
