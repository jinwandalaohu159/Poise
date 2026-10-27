using System.IO.Pipes;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Poise.Core.Ipc;

/// <summary>服务不可用（未安装、未运行或连接断开）。</summary>
public sealed class ServiceUnavailableException(string message, Exception? inner = null) : Exception(message, inner);

/// <summary>服务返回了错误（例如观察模式下拒绝修改）。<see cref="Exception.Message"/> 是给日志看的原文。</summary>
public sealed class ServiceErrorException(ErrorCode code, string message) : Exception(message)
{
    public ErrorCode Code { get; } = code;
}

/// <summary>
/// 与 Poise 服务通信的命名管道客户端：一行一条 JSON，请求-响应串行进行，断开后自动重连。
/// </summary>
public sealed class PoiseClient : IDisposable
{
    private const string PipeName = "Poise";
    private static readonly UTF8Encoding Utf8 = new(false);

    private readonly SemaphoreSlim _lock = new(1, 1);
    private NamedPipeClientStream? _pipe;
    private StreamReader? _reader;
    private StreamWriter? _writer;
    private ulong _nextId;
    private bool _connected;

    public event Action<bool>? ConnectionChanged;

    public bool IsConnected => _connected;

    /// <summary>握手时服务报告的版本号。</summary>
    public string? ServiceVersion { get; private set; }

    /// <summary>没有返回值的调用。</summary>
    public Task CallAsync(string method, object? parameters = null, CancellationToken ct = default) =>
        CallAsync<JsonElement>(method, parameters, ct);

    public async Task<T> CallAsync<T>(string method, object? parameters = null, CancellationToken ct = default)
    {
        await _lock.WaitAsync(ct).ConfigureAwait(false);
        try
        {
            await EnsureConnectedAsync(ct).ConfigureAwait(false);
            try
            {
                var result = await SendAsync(method, parameters, ct).ConfigureAwait(false);
                return result.Deserialize<T>(ProtocolJson.Options)!;
            }
            catch (IOException e)
            {
                Disconnect();
                throw new ServiceUnavailableException("与服务的连接已断开", e);
            }
        }
        finally
        {
            _lock.Release();
        }
    }

    private async Task<JsonElement> SendAsync(string method, object? parameters, CancellationToken ct)
    {
        var request = new JsonObject { ["method"] = method };
        if (parameters is not null)
        {
            request["params"] = JsonSerializer.SerializeToNode(parameters, parameters.GetType(), ProtocolJson.Options);
        }
        var envelope = new JsonObject { ["id"] = ++_nextId, ["request"] = request };
        await _writer!.WriteLineAsync(envelope.ToJsonString().AsMemory(), ct).ConfigureAwait(false);
        var line = await _reader!.ReadLineAsync(ct).ConfigureAwait(false)
                   ?? throw new IOException("服务关闭了连接");
        using var doc = JsonDocument.Parse(line);
        var root = doc.RootElement;
        if (!root.GetProperty("ok").GetBoolean())
        {
            var error = root.TryGetProperty("error", out var e) ? e.GetString() : null;
            throw new ServiceErrorException(ParseCode(root), error ?? "服务返回错误");
        }
        return root.TryGetProperty("result", out var r) ? r.Clone() : default;
    }

    /// <summary>较新的服务可能返回界面还不认识的代码，这时按"未知"处理。</summary>
    internal static ErrorCode ParseCode(JsonElement root)
    {
        if (!root.TryGetProperty("code", out var c) || c.ValueKind != JsonValueKind.String)
        {
            return ErrorCode.Unknown;
        }
        try
        {
            return c.Deserialize<ErrorCode>(ProtocolJson.Options);
        }
        catch (JsonException)
        {
            return ErrorCode.Unknown;
        }
    }

    private async Task EnsureConnectedAsync(CancellationToken ct)
    {
        if (_pipe is { IsConnected: true })
        {
            return;
        }
        Disconnect();
        var pipe = new NamedPipeClientStream(".", PipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        try
        {
            await pipe.ConnectAsync(500, ct).ConfigureAwait(false);
        }
        catch (Exception e) when (e is TimeoutException or IOException or UnauthorizedAccessException)
        {
            await pipe.DisposeAsync().ConfigureAwait(false);
            throw new ServiceUnavailableException("Poise 服务没有运行", e);
        }
        _pipe = pipe;
        // 读写器不拥有管道（leaveOpen），管道只由 Disconnect 关闭一次。
        _reader = new StreamReader(pipe, Utf8, detectEncodingFromByteOrderMarks: false, leaveOpen: true);
        _writer = new StreamWriter(pipe, Utf8, leaveOpen: true) { AutoFlush = true, NewLine = "\n" };
        var hello = await SendAsync("hello", new { protocol = ProtocolJson.Version }, ct).ConfigureAwait(false);
        ServiceVersion = hello.Deserialize<HelloResult>(ProtocolJson.Options)?.ServiceVersion;
        SetConnected(true);
    }

    /// <summary>
    /// 关闭管道。读写器不持有资源，不单独释放：写入器释放时会把缓冲写进管道，
    /// 管道已断开（服务退出）时会抛异常。
    /// </summary>
    private void Disconnect()
    {
        _pipe?.Dispose();
        _reader = null;
        _writer = null;
        _pipe = null;
        SetConnected(false);
    }

    private void SetConnected(bool value)
    {
        if (_connected == value)
        {
            return;
        }
        _connected = value;
        ConnectionChanged?.Invoke(value);
    }

    public void Dispose()
    {
        Disconnect();
        _lock.Dispose();
    }
}
