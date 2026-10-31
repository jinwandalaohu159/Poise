using System.Text.Json;
using System.Text.Json.Serialization;

namespace Poise.Core.Ipc;

/// <summary>一条回复：编号、是否成功、结果或失败的原因（见 Rust 端 ResponseEnvelope）。</summary>
internal sealed class Reply<T>
{
    public ulong Id { get; init; }
    public bool Ok { get; init; }
    public T? Result { get; init; }
    /// <summary>失败分类；较新的服务可能给出界面不认识的值（见 <see cref="PipeChannel.ParseCode(JsonElement)"/>）。</summary>
    public JsonElement Code { get; init; }
    public string? Error { get; init; }
}

/// <summary>
/// 编译时生成的读写代码：界面启动时、每次读回复时不再靠反射摸索类型（第一次读状态更快，也少分配）。
/// 没列在这里的类型（请求参数用的匿名类型）仍按反射处理，见 <see cref="ProtocolJson.Options"/>。
/// </summary>
[JsonSourceGenerationOptions(
    PropertyNamingPolicy = JsonKnownNamingPolicy.SnakeCaseLower,
    DefaultIgnoreCondition = JsonIgnoreCondition.Never)]
[JsonSerializable(typeof(Reply<JsonElement>))]
[JsonSerializable(typeof(Reply<HelloResult>))]
[JsonSerializable(typeof(Reply<Status>))]
[JsonSerializable(typeof(Reply<Trend>))]
[JsonSerializable(typeof(Reply<CoreMap>))]
[JsonSerializable(typeof(Reply<List<AppEntry>>))]
[JsonSerializable(typeof(Reply<CleanupOverview>))]
[JsonSerializable(typeof(Reply<CleanupResult>))]
[JsonSerializable(typeof(Reply<List<ItemStatus>>))]
[JsonSerializable(typeof(Reply<ApplyReport>))]
[JsonSerializable(typeof(Reply<List<string>>))]
[JsonSerializable(typeof(Reply<Diagnosis>))]
[JsonSerializable(typeof(Reply<EventPage>))]
[JsonSerializable(typeof(Reply<List<SessionRecord>>))]
internal sealed partial class ProtocolContext : JsonSerializerContext;
