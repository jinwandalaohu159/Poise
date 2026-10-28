using Poise.Core.Ipc;
using Poise.Core.Monitoring;
using Poise.Core.Presentation;

namespace Poise.App.UI.ViewModels;

/// <summary>卡顿诊断页：分析，以及执行可以直接完成的建议（跳转类建议由页面处理）。</summary>
public sealed class DiagnoseViewModel(AppHost host)
{
    /// <summary>分析一次。服务没连上或还在收集数据时返回 null，<paramref name="error"/> 为原因。</summary>
    public async Task<(DiagnoseView? View, string? Error)> AnalyzeAsync()
    {
        try
        {
            return (DiagnoseText.Build(await host.Api.DiagnoseAsync()), null);
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            return (null, ErrorText.For(e));
        }
    }

    /// <summary>执行"设为低优先级""切换至全力模式"。成功返回 null，否则返回给用户看的原因。</summary>
    public async Task<string?> ApplyAsync(Suggestion s)
    {
        try
        {
            switch (s.Kind)
            {
                case SuggestionKind.SetLowPriority when s.Path is { } path:
                    await host.Api.SetAppPreferenceAsync(path, AppPreference.Background);
                    break;
                case SuggestionKind.SwitchToFull:
                    await host.Status.ChangeAsync(() => host.Api.SetModeAsync(Mode.Full), s => ExpectedStatus.WithMode(s, Mode.Full));
                    break;
            }
            return null;
        }
        catch (Exception e) when (e is ServiceUnavailableException or ServiceErrorException)
        {
            return ErrorText.For(e);
        }
    }
}
