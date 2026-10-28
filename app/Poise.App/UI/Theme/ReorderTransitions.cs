using System.Numerics;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Hosting;

namespace Poise.App.UI.Theme;

/// <summary>
/// 虚拟化列表（ItemsRepeater）的数据变化动画：重新排序时，其余的行从原位置平滑移到新位置，
/// 换了位置的行在新位置淡入。全部在合成线程上执行。
/// 移走的行立即消失、不做淡出：列表把"换位置"当作"在旧位置删除 + 在新位置插入"，
/// 淡出的旧行会和滑过来的行叠在一起，文字重影。
/// 只处理数据变化（排序、程序启动或退出），不处理布局变化：展开某一行时，下方的行由布局逐帧跟着走，不需要再加动画。
/// </summary>
public sealed partial class ReorderTransitions : ItemCollectionTransitionProvider
{
    private const ItemCollectionTransitionTriggers DataChanges =
        ItemCollectionTransitionTriggers.CollectionChangeAdd
        | ItemCollectionTransitionTriggers.CollectionChangeRemove
        | ItemCollectionTransitionTriggers.CollectionChangeReset;

    protected override bool ShouldAnimateCore(ItemCollectionTransition transition) =>
        Motion.Enabled && (transition.Triggers & DataChanges) != 0;

    protected override void StartTransitions(IList<ItemCollectionTransition> transitions)
    {
        foreach (var transition in transitions)
        {
            var progress = transition.Start();
            if (transition.Operation == ItemCollectionTransitionOperation.Remove)
            {
                progress.Complete();
                continue;
            }
            var element = progress.Element;
            var visual = ElementCompositionPreview.GetElementVisual(element);
            var compositor = visual.Compositor;
            var batch = compositor.CreateScopedBatch(CompositionBatchTypes.Animation);
            switch (transition.Operation)
            {
                case ItemCollectionTransitionOperation.Move:
                    ElementCompositionPreview.SetIsTranslationEnabled(element, true);
                    var move = compositor.CreateVector3KeyFrameAnimation();
                    var dy = (float)(transition.OldBounds.Y - transition.NewBounds.Y);
                    move.InsertKeyFrame(0, new Vector3(0, dy, 0));
                    move.InsertKeyFrame(1, Vector3.Zero, Decelerate(compositor));
                    move.Duration = Motion.Normal;
                    visual.StartAnimation("Translation", move);
                    break;
                case ItemCollectionTransitionOperation.Add:
                    visual.StartAnimation("Opacity", Fade(compositor, 0, 1, Motion.Fast));
                    break;
            }
            batch.End();
            batch.Completed += (_, _) =>
            {
                progress.Complete();
            };
        }
    }

    private static ScalarKeyFrameAnimation Fade(Compositor compositor, float from, float to, TimeSpan duration)
    {
        var fade = compositor.CreateScalarKeyFrameAnimation();
        fade.InsertKeyFrame(0, from);
        fade.InsertKeyFrame(1, to);
        fade.Duration = duration;
        return fade;
    }

    /// <summary>Fluent 的减速曲线：快速起步，平稳停下。</summary>
    private static CompositionEasingFunction Decelerate(Compositor compositor) =>
        compositor.CreateCubicBezierEasingFunction(new Vector2(0.1f, 0.9f), new Vector2(0.2f, 1f));
}
