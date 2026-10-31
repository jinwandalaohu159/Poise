using System.Text;
using System.Text.Json;
using Poise.Core.Ipc;

namespace Poise.Core.Tests;

/// <summary>管道上的一行一条回复：按行读出字节，直接读成结果类型。</summary>
public sealed class IpcTests
{
    /// <summary>每次最多读出几个字节，模拟回复分几次到达。</summary>
    private sealed class Trickle(byte[] data, int chunk) : Stream
    {
        private int _at;

        public override int Read(byte[] buffer, int offset, int count)
        {
            var n = Math.Min(Math.Min(count, chunk), data.Length - _at);
            Array.Copy(data, _at, buffer, offset, n);
            _at += n;
            return n;
        }

        public override bool CanRead => true;
        public override bool CanSeek => false;
        public override bool CanWrite => false;
        public override long Length => data.Length;
        public override long Position { get => _at; set => throw new NotSupportedException(); }
        public override void Flush() { }
        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
        public override void SetLength(long value) => throw new NotSupportedException();
        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();
    }

    [Fact]
    public async Task Lines_arriving_in_pieces_and_longer_than_the_buffer_are_read_whole()
    {
        var big = new string('x', 40_000);
        var data = Encoding.UTF8.GetBytes($"first\n{big}\r\nlast\n");
        var reader = new LineReader(new Trickle(data, 7_000));

        Assert.Equal("first", Encoding.UTF8.GetString((await reader.ReadLineAsync(default))!.Value.Span));
        Assert.Equal(big, Encoding.UTF8.GetString((await reader.ReadLineAsync(default))!.Value.Span));
        Assert.Equal("last", Encoding.UTF8.GetString((await reader.ReadLineAsync(default))!.Value.Span));
        Assert.Null(await reader.ReadLineAsync(default));
    }

    [Fact]
    public void A_reply_is_read_straight_into_its_result_type()
    {
        var line = $$"""{"id":3,"ok":true,"result":{{TestSupport.FixtureText("status_full.json")}}}""";

        var reply = JsonSerializer.Deserialize<Reply<Status>>(Encoding.UTF8.GetBytes(line), ProtocolJson.Options)!;

        Assert.Equal((3UL, true), (reply.Id, reply.Ok));
        Assert.Equal(TestSupport.Fixture<Status>("status_full.json").Health, reply.Result!.Health);
    }

    [Fact]
    public void A_failed_reply_carries_its_error_code()
    {
        var bytes = Encoding.UTF8.GetBytes(TestSupport.FixtureText("error_response.json"));

        var reply = JsonSerializer.Deserialize<Reply<JsonElement>>(bytes, ProtocolJson.Options)!;

        Assert.False(reply.Ok);
        Assert.Equal(ErrorCode.ObserveOnly, PipeChannel.ParseCode(reply.Code));
        Assert.Equal("dry run", reply.Error);
    }

    [Fact]
    public void Requests_are_written_with_snake_case_fields_and_values()
    {
        Assert.Equal("""{"id":3,"request":{"method":"set_mode","params":{"mode":"quiet"}}}""",
            PipeChannel.Encode(3, "set_mode", new ModeParams(Mode.Quiet)));
        Assert.Equal("""{"id":4,"request":{"method":"set_app_preference","params":{"path":"C:\\a.exe","preference":"always_fast"}}}""",
            PipeChannel.Encode(4, "set_app_preference", new AppPreferenceParams(@"C:\a.exe", AppPreference.AlwaysFast)));
        Assert.Equal("""{"id":5,"request":{"method":"get_status"}}""", PipeChannel.Encode(5, "get_status", null));
    }
}
