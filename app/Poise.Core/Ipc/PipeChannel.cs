using System.Buffers;
using System.IO.Pipes;
using System.Text;
using System.Text.Json;

namespace Poise.Core.Ipc;

/// <summary>
/// 到服务的一条命名管道连接：一行一条 JSON，请求-响应串行进行，断开后下次调用时重连。
/// 每次调用都有时限：服务卡住时不一直等下去。超时或取消时断开连接——回复可能还在路上，
/// 留着连接的话下一次调用会读到上一次的回复；回复的编号对不上时同样断开。
/// 回复直接从字节读成结果类型（见 <see cref="LineReader"/>、<see cref="ProtocolContext"/>）。
/// 连不上服务（没有运行）时，之后一小段时间里直接失败、不再去连：每次连接要等半秒，
/// 状态、走势、页面的刷新排在后面一起等，用户的操作也被挡住。
/// </summary>
internal sealed class PipeChannel : IDisposable
{
    private const string PipeName = "Poise";
    private static readonly UTF8Encoding Utf8 = new(false);
    /// <summary>
    /// 连不上之后多久再试；连续连不上时逐次加倍，最多到 <see cref="MaxRetryAfter"/>（服务启动好之后，
    /// 界面最多晚两秒连上）。窗口重新显示时不等，立即再试（<see cref="RetryNow"/>）。
    /// </summary>
    private static readonly TimeSpan FirstRetryAfter = TimeSpan.FromMilliseconds(500);
    private static readonly TimeSpan MaxRetryAfter = TimeSpan.FromSeconds(2);

    private readonly SemaphoreSlim _lock = new(1, 1);
    private NamedPipeClientStream? _pipe;
    private LineReader? _reader;
    private StreamWriter? _writer;
    private ulong _nextId;
    private bool _connected;
    /// <summary>连不上时：这之前不再去连（<see cref="Environment.TickCount64"/>）。</summary>
    private long _retryAt;
    private TimeSpan _retryAfter = FirstRetryAfter;

    public bool IsConnected => _connected;

    /// <summary>下次调用立即去连，不等重连间隔（窗口刚显示出来，用户在看）。</summary>
    public void RetryNow() => Interlocked.Exchange(ref _retryAt, 0);

    /// <summary>握手时服务报告的版本号。</summary>
    public string? ServiceVersion { get; private set; }

    public async Task<T?> CallAsync<T>(string method, object? parameters, TimeSpan timeout, CancellationToken ct)
    {
        // 时限从排队等前面的请求就开始算：服务卡住时请求不会越排越长、用户的操作不会等上几分钟。
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(ct);
        deadline.CancelAfter(timeout);
        try
        {
            await _lock.WaitAsync(deadline.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException e) when (!ct.IsCancellationRequested)
        {
            throw new ServiceUnavailableException("服务没有及时回复（前面的请求还没完成）", e);
        }
        try
        {
            await EnsureConnectedAsync(deadline.Token).ConfigureAwait(false);
            return await SendAsync<T>(method, parameters, deadline.Token).ConfigureAwait(false);
        }
        catch (JsonException e) when (_connected)
        {
            // 整行已经读完，连接没有错位：只是这一次的回复界面不认识（版本错位、新的取值），不断开——
            // 断开的话每次查询都会重连、显示"未连接"。
            throw new ServiceErrorException(ErrorCode.Unknown, $"服务对 {method} 的回复无法解析：{e.Message}");
        }
        catch (JsonException e)
        {
            // 握手的回复解析不了：EnsureConnectedAsync 已经断开，按连不上处理。
            throw new ServiceUnavailableException("服务的握手回复无法解析", e);
        }
        catch (Exception e) when (e is IOException or ObjectDisposedException)
        {
            Disconnect();
            throw new ServiceUnavailableException("与服务的连接已断开", e);
        }
        catch (OperationCanceledException e)
        {
            Disconnect();
            if (ct.IsCancellationRequested)
            {
                throw;
            }
            throw new ServiceUnavailableException("服务没有及时回复", e);
        }
        finally
        {
            _lock.Release();
        }
    }

    private async Task<T?> SendAsync<T>(string method, object? parameters, CancellationToken ct)
    {
        var id = ++_nextId;
        await _writer!.WriteLineAsync(Encode(id, method, parameters).AsMemory(), ct).ConfigureAwait(false);
        var line = await _reader!.ReadLineAsync(ct).ConfigureAwait(false)
                   ?? throw new IOException("服务关闭了连接");
        var reply = JsonSerializer.Deserialize<Reply<T>>(line.Span, ProtocolJson.Options)
                    ?? throw new IOException("回复是空的");
        if (reply.Id != id)
        {
            throw new IOException("回复与请求不对应");
        }
        if (!reply.Ok)
        {
            throw new ServiceErrorException(ParseCode(reply.Code), reply.Error ?? "服务返回错误");
        }
        return reply.Result;
    }

    /// <summary>
    /// 一条请求（见 Rust 端 RequestEnvelope）：<c>{"id":…,"request":{"method":…,"params":…}}</c>。直接写成 UTF-8，
    /// 参数按编译时生成的代码写（见 <see cref="ProtocolContext"/>），不经过文档树。
    /// </summary>
    internal static string Encode(ulong id, string method, object? parameters)
    {
        var buffer = new ArrayBufferWriter<byte>(256);
        using (var w = new Utf8JsonWriter(buffer))
        {
            w.WriteStartObject();
            w.WriteNumber("id", id);
            w.WriteStartObject("request");
            w.WriteString("method", method);
            if (parameters is not null)
            {
                w.WritePropertyName("params");
                JsonSerializer.Serialize(w, parameters, ProtocolJson.Options.GetTypeInfo(parameters.GetType()));
            }
            w.WriteEndObject();
            w.WriteEndObject();
        }
        return Utf8.GetString(buffer.WrittenSpan);
    }

    /// <summary>较新的服务可能返回界面还不认识的代码，这时按"未知"处理。<paramref name="code"/> 是回复里的 code。</summary>
    internal static ErrorCode ParseCode(JsonElement code)
    {
        if (code.ValueKind != JsonValueKind.String)
        {
            return ErrorCode.Unknown;
        }
        try
        {
            return code.Deserialize<ErrorCode>(ProtocolJson.Options);
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
        if (Environment.TickCount64 < _retryAt)
        {
            throw new ServiceUnavailableException("Poise 服务没有运行");
        }
        var pipe = new NamedPipeClientStream(".", PipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        try
        {
            await pipe.ConnectAsync(500, ct).ConfigureAwait(false);
        }
        catch (Exception e) when (e is TimeoutException or IOException or UnauthorizedAccessException)
        {
            await pipe.DisposeAsync().ConfigureAwait(false);
            _retryAt = Environment.TickCount64 + (long)_retryAfter.TotalMilliseconds;
            _retryAfter = TimeSpan.FromTicks(Math.Min(_retryAfter.Ticks * 2, MaxRetryAfter.Ticks));
            throw new ServiceUnavailableException("Poise 服务没有运行", e);
        }
        _retryAt = 0;
        _retryAfter = FirstRetryAfter;
        _pipe = pipe;
        // 读写器不拥有管道（leaveOpen），管道只由 Disconnect 关闭一次。
        _reader = new LineReader(pipe);
        _writer = new StreamWriter(pipe, Utf8, leaveOpen: true) { AutoFlush = true, NewLine = "\n" };
        // 握手成功、协议版本一致才算连上；否则断开（不留着一条没握手的连接给下一次调用用）。
        try
        {
            var hello = await SendAsync<HelloResult>("hello", new HelloParams(ProtocolJson.Version), ct).ConfigureAwait(false);
            if (hello is null || hello.Protocol != ProtocolJson.Version)
            {
                throw new ServiceErrorException(ErrorCode.VersionMismatch, "服务的协议版本与界面不一致");
            }
            ServiceVersion = hello.ServiceVersion;
        }
        catch
        {
            Disconnect();
            _retryAt = Environment.TickCount64 + (long)_retryAfter.TotalMilliseconds;
            _retryAfter = TimeSpan.FromTicks(Math.Min(_retryAfter.Ticks * 2, MaxRetryAfter.Ticks));
            throw;
        }
        _connected = true;
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
        _connected = false;
    }

    /// <summary>
    /// 只关管道：还在路上的请求随之以"连不上"结束（见 <see cref="CallAsync{T}"/>）。信号量不释放——
    /// 那个请求结束时还要归还它。
    /// </summary>
    public void Dispose() => _pipe?.Dispose();
}
