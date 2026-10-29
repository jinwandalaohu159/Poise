// 优化页：优化项与应用结果。
using System.Text.Json;

namespace Poise.Core.Ipc;

/// <summary>文案：type 为 key（本地化键）或 plain（原文）。</summary>
public sealed record TextValue(string Type, string Value);

public sealed record OptimizationItem(
    string Id,
    string Group,
    TextValue Title,
    TextValue Description,
    bool InOneClick,
    bool SlowHint,
    string TakesEffect);

public sealed record ItemStatus(OptimizationItem Item, string State, bool Revertible)
{
    public bool IsOptimized => State == "optimized";
}

public sealed record ApplyReport(List<string> Applied, List<JsonElement> Failed);
