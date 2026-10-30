using Microsoft.UI.Xaml.Controls;
using Poise.App.UI.Controls;
using Poise.App.UI.ViewModels;
using Poise.Core.Localization;

namespace Poise.App.UI.Pages;

/// <summary>
/// 核心与缓存面板：共用一块三级缓存的几组核心圈在一起，每组写明二级缓存、负载，画出每个核心和它的线程，
/// 并列出 Poise 放在这组核心上的程序；最后是没有限制核心、由 Windows 安排的程序。
/// 读取与刷新都在 <see cref="CoresViewModel"/> 里，这里只管页面的显示与隐藏。
/// </summary>
public sealed partial class CoresPage : Page
{
    public CoresPage()
    {
        InitializeComponent();
        Header.Title = Loc.Get("cores.title");
        Vm.Error += ErrorBar.ShowError;
        Loaded += (_, _) => Vm.Start();
        Unloaded += (_, _) => Vm.Stop();
    }

    public CoresViewModel Vm { get; } = new(AppHost.Current);
}
