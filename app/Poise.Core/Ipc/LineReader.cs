namespace Poise.Core.Ipc;

/// <summary>
/// 从管道按行读出 UTF-8 字节，不转成字符串：大的回复（程序列表几十 KB）直接从字节反序列化，
/// 不在大对象堆上反复分配字符串；缓冲区一直复用（放不下时加倍）。为一次特别大的回复加大的缓冲区，
/// 读完之后（里面没有剩下的）缩回原来的大小：界面常驻，不一直占着几百 KB。
/// </summary>
internal sealed class LineReader(Stream stream)
{
    private const int InitialSize = 16 * 1024;
    /// <summary>缓冲区超过这么大、又已经读空时缩回 <see cref="InitialSize"/>。</summary>
    private const int ShrinkAbove = 128 * 1024;

    private byte[] _buffer = new byte[InitialSize];
    private int _start;
    private int _end;

    /// <summary>下一行（不含换行），在下次调用之前有效；对方关闭了连接时为 null。</summary>
    public async ValueTask<ReadOnlyMemory<byte>?> ReadLineAsync(CancellationToken ct)
    {
        // 上一行在这次调用之前一直有效，所以到这里（调用方已经用完）才缩。
        if (_start == _end && _buffer.Length > ShrinkAbove)
        {
            _buffer = new byte[InitialSize];
            _start = _end = 0;
        }
        var searched = _start;
        while (true)
        {
            var newline = _buffer.AsSpan(searched, _end - searched).IndexOf((byte)'\n');
            if (newline >= 0)
            {
                var end = searched + newline;
                var line = _buffer.AsMemory(_start, end - _start);
                _start = end + 1;
                return line.Span.EndsWith("\r"u8) ? line[..^1] : line;
            }
            searched = _end;
            if (_start > 0)
            {
                // 读过的部分不要了：没读完的挪到开头。
                Buffer.BlockCopy(_buffer, _start, _buffer, 0, _end - _start);
                searched -= _start;
                _end -= _start;
                _start = 0;
            }
            if (_end == _buffer.Length)
            {
                Array.Resize(ref _buffer, _buffer.Length * 2);
            }
            var read = await stream.ReadAsync(_buffer.AsMemory(_end), ct).ConfigureAwait(false);
            if (read == 0)
            {
                return null;
            }
            _end += read;
        }
    }
}
