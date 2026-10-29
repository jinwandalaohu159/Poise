// 与 Rust 端 poise-protocol 一一对应的数据结构（按界面的页面分在本目录的几个文件里）。字段名统一为 snake_case。
// 这里是序列化选项、各页共用的枚举和失败分类。
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Poise.Core.Ipc;

public static class ProtocolJson
{
    public const int Version = 1;

    public static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        DefaultIgnoreCondition = JsonIgnoreCondition.Never,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.SnakeCaseLower) },
    };
}

/// <summary>模式。实际生效的模式不会是 Auto；Vendor 是机型极限模式（戴尔 G 模式等）。</summary>
public enum Mode { Auto, Quiet, Balanced, Full, Vendor }

/// <summary>玩游戏时切换到哪个模式。</summary>
public enum GameMode { Off, Full, Vendor }

public enum AppPreference { Auto, AlwaysFast, Background, Game }

/// <summary>服务返回的失败分类。界面按它显示对应文案，服务给的文字只写进日志。</summary>
public enum ErrorCode
{
    Unknown,
    InvalidRequest,
    VersionMismatch,
    ObserveOnly,
    NotFound,
    ProcessGone,
    InvalidPath,
    NotReady,
    Failed,
    Internal,
}
