using System.Text.Json.Serialization;
using Poise.Core.Settings;

namespace Poise.Core;

/// <summary>
/// 本地 JSON（界面设置、文字表）的读写代码在编译时生成：界面启动时不再靠反射摸索类型。
/// 和反射读写的格式完全相同（托盘代理也读界面设置）。
/// </summary>
[JsonSerializable(typeof(UiSettings))]
[JsonSerializable(typeof(Dictionary<string, string>))]
internal sealed partial class LocalJson : JsonSerializerContext;
