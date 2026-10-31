namespace Poise.Core.Ipc;

// 请求参数（与 Rust 端 Request 的各个变体对应）。都列在 ProtocolContext 里，按编译时生成的代码写，
// 不靠反射（第一次调用不用现场生成读写代码）。

internal sealed record HelloParams(int Protocol);

internal sealed record VisibleParams(bool Visible);

internal sealed record ModeParams(Mode Mode);

internal sealed record GameModeParams(GameMode Mode);

internal sealed record EnabledParams(bool Enabled);

internal sealed record FanBoostParams(byte Percent);

internal sealed record AppPreferenceParams(string Path, AppPreference Preference);

internal sealed record IdParams(string Id);

internal sealed record EventsParams(ulong After);
