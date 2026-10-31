using System.Text.Json;

namespace Poise.Core.Ipc;

/// <summary>服务不可用（未安装、未运行、连接断开或没有及时回复）。</summary>
public sealed class ServiceUnavailableException(string message, Exception? inner = null) : Exception(message, inner);

/// <summary>服务返回了错误（例如观察模式下拒绝修改）。<see cref="Exception.Message"/> 是给日志看的原文。</summary>
public sealed class ServiceErrorException(ErrorCode code, string message) : Exception(message)
{
    public ErrorCode Code { get; } = code;
}

/// <summary>
/// 与 Poise 服务通信：两条连接（见 <see cref="PipeChannel"/>），断开后自动重连。
/// 要等很久的操作（一键清理要等清理执行完，列出、应用优化项要读写一批系统设置）走单独的一条，
/// 不挡住每秒一次的状态查询和页面的刷新。
/// </summary>
public sealed class PoiseClient : IDisposable
{
    /// <summary>一般请求最多等这么久。</summary>
    private static readonly TimeSpan Timeout = TimeSpan.FromSeconds(10);
    /// <summary>慢的操作最多等这么久（重启服务最多要等十几秒，电源服务忙时读写设置也要好几秒）。</summary>
    private static readonly TimeSpan SlowTimeout = TimeSpan.FromSeconds(90);

    private static readonly HashSet<string> SlowMethods =
    [
        "clean_now", "list_optimizations", "apply_optimization", "revert_optimization", "apply_recommended", "revert_all",
    ];

    private readonly PipeChannel _live = new();
    private readonly PipeChannel _slow = new();

    public bool IsConnected => _live.IsConnected;

    /// <summary>连不上服务时，下次调用立即再试（见 <see cref="PipeChannel.RetryNow"/>）。</summary>
    public void RetryNow()
    {
        _live.RetryNow();
        _slow.RetryNow();
    }

    /// <summary>握手时服务报告的版本号。</summary>
    public string? ServiceVersion => _live.ServiceVersion ?? _slow.ServiceVersion;

    /// <summary>没有返回值的调用。</summary>
    public Task CallAsync(string method, object? parameters = null, CancellationToken ct = default) =>
        CallAsync<JsonElement>(method, parameters, ct);

    public async Task<T> CallAsync<T>(string method, object? parameters = null, CancellationToken ct = default)
    {
        var slow = SlowMethods.Contains(method);
        var result = await (slow ? _slow : _live)
            .CallAsync<T>(method, parameters, slow ? SlowTimeout : Timeout, ct)
            .ConfigureAwait(false);
        return result!;
    }

    public void Dispose()
    {
        _live.Dispose();
        _slow.Dispose();
    }
}
