namespace Poise.Core.Monitoring;

/// <summary>
/// 协调"定期查询"与"用户修改"，避免界面跳回旧值：
/// <list type="bullet">
/// <item>修改进行中、或查询期间发生过修改时，这次查询的结果可能是旧的，丢弃不发布；</item>
/// <item>查询进行中又收到刷新请求，不丢弃，等当前这次结束后再查一次。</item>
/// </list>
/// 只在界面线程上使用（不加锁）。
/// </summary>
public sealed class RefreshGate
{
    private bool _busy;
    private bool _again;
    /// <summary>每次修改开始和结束都加一；查询前后不一致说明结果可能已过时。</summary>
    private long _version;
    private int _changing;

    /// <summary>执行一次修改。完成后由调用方决定是否刷新。</summary>
    public async Task ChangeAsync(Func<Task> change)
    {
        _changing++;
        _version++;
        try
        {
            await change();
        }
        finally
        {
            _changing--;
            _version++;
        }
    }

    /// <summary>
    /// 查询并发布。<paramref name="query"/> 返回"如何应用结果"，只有结果仍然有效时才会执行。
    /// </summary>
    public async Task RefreshAsync(Func<Task<Action>> query)
    {
        if (_busy)
        {
            _again = true;
            return;
        }
        _busy = true;
        try
        {
            do
            {
                _again = false;
                var version = _version;
                var apply = await query();
                if (_changing == 0 && version == _version)
                {
                    apply();
                }
                else
                {
                    // 修改完成后调用方会再刷新；这里补一次，保证最终显示的是最新状态。
                    _again = true;
                }
            }
            while (_again && _changing == 0);
        }
        finally
        {
            _busy = false;
        }
    }
}
