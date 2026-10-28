using System.Runtime.InteropServices;

namespace Poise.App.UI.Windows;

/// <summary>
/// 监视前台窗口的变化（点桌面、点任务栏、切到其他程序）。托盘面板显示期间开启，用来"点别处就收起"：
/// 即使面板没能成为前台窗口（收不到 Deactivated），前台一变化也能收起。
/// 回调在调用 <see cref="Start"/> 的线程（界面线程）上触发。
/// </summary>
internal sealed class ForegroundWatcher : IDisposable
{
    private const uint EventSystemForeground = 0x0003;
    private const uint WinEventOutOfContext = 0x0000;

    private readonly WinEventProc _callback;
    private IntPtr _hook;

    public ForegroundWatcher() => _callback = OnEvent;

    private delegate void WinEventProc(IntPtr hook, uint eventType, IntPtr hwnd, int idObject, int idChild, uint thread, uint time);

    /// <summary>新的前台窗口句柄。</summary>
    public event Action<IntPtr>? Changed;

    public void Start()
    {
        if (_hook == IntPtr.Zero)
        {
            _hook = SetWinEventHook(EventSystemForeground, EventSystemForeground, IntPtr.Zero, _callback, 0, 0, WinEventOutOfContext);
        }
    }

    public void Stop()
    {
        if (_hook != IntPtr.Zero)
        {
            UnhookWinEvent(_hook);
            _hook = IntPtr.Zero;
        }
    }

    public void Dispose() => Stop();

    private void OnEvent(IntPtr hook, uint eventType, IntPtr hwnd, int idObject, int idChild, uint thread, uint time) => Changed?.Invoke(hwnd);

    [DllImport("user32.dll")]
    private static extern IntPtr SetWinEventHook(uint eventMin, uint eventMax, IntPtr module, WinEventProc callback, uint processId, uint threadId, uint flags);

    [DllImport("user32.dll")]
    private static extern bool UnhookWinEvent(IntPtr hook);
}
