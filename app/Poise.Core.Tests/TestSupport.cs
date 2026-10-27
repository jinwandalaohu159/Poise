using System.Text.Json;
using Poise.Core.Ipc;
using Poise.Core.Localization;

// 文案语言是全局状态（Loc），测试之间不能并行切换。
[assembly: CollectionBehavior(DisableTestParallelization = true)]

namespace Poise.Core.Tests;

/// <summary>测试共用：读取契约样本、固定界面语言。</summary>
internal static class TestSupport
{
    private static string Dir(string name) => Path.Combine(AppContext.BaseDirectory, name);

    public static string FixtureText(string name) => File.ReadAllText(Path.Combine(Dir("Fixtures"), name));

    public static T Fixture<T>(string name) =>
        JsonSerializer.Deserialize<T>(FixtureText(name), ProtocolJson.Options)
        ?? throw new InvalidDataException($"样本 {name} 为空");

    public static Dictionary<string, string> StringTable(string language) =>
        JsonSerializer.Deserialize<Dictionary<string, string>>(File.ReadAllText(Path.Combine(Dir("Strings"), $"{language}.json")))!;

    /// <summary>依赖文案的测试在开始时设定语言，不依赖前一个测试留下的状态。</summary>
    public static void UseChinese() => Loc.SetLanguage(Loc.Chinese);

    public static void UseEnglish() => Loc.SetLanguage(Loc.English);
}
