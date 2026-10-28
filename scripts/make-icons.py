"""由 icon/foam.png（512×512，透明底）生成 Poise 的全部图标。

    python scripts/make-icons.py

输出到 app/Poise.App/Assets/：
- Poise.ico           程序与托盘图标（16～256 多尺寸）
- PoiseAttention.ico  托盘"需要注意"：右下角加橙色圆点
- Poise.png           界面里的标志（标题栏、设置页）

原图是细线条，直接缩到托盘尺寸时线宽不到 1 像素、发虚，细节也挤成一团。所以按尺寸分级：
- 32px 以上：完整图案，必要时略加粗；
- 24px 以下：只保留三个泡泡的轮廓（去掉内部高光和星星），线条加粗到约 1.6 像素。
颜色始终取自原图的渐变，不因加粗而偏色。依赖 Pillow。
"""

from collections import deque
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'icon' / 'foam.png'
ASSETS = ROOT / 'app' / 'Poise.App' / 'Assets'

SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
# 不大于这个尺寸时只画主体轮廓。
SIMPLE_MAX = 24
# 原图线宽（512px 下约 22px）与缩放后希望达到的最小线宽。
SOURCE_STROKE = 22
MIN_STROKE = 1.6
# Windows 的提醒色（与界面里的 Caution 一致）。
ATTENTION = (247, 99, 12, 255)


def components(alpha: Image.Image) -> list[list[int]]:
    """连通区域（像素下标列表），按面积从大到小。"""
    w, h = alpha.size
    px = alpha.load()
    seen = bytearray(w * h)
    found = []
    for start in range(w * h):
        if seen[start] or px[start % w, start // w] < 32:
            continue
        seen[start] = 1
        region, queue = [], deque([start])
        while queue:
            i = queue.popleft()
            region.append(i)
            x, y = i % w, i // w
            for nx, ny in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
                if 0 <= nx < w and 0 <= ny < h:
                    j = ny * w + nx
                    if not seen[j] and px[nx, ny] >= 32:
                        seen[j] = 1
                        queue.append(j)
        found.append(region)
    return sorted(found, key=len, reverse=True)


def without(alpha: Image.Image, regions: list[list[int]]) -> Image.Image:
    """把这些区域从图中去掉（连同抗锯齿的边缘一起）。"""
    w = alpha.width
    cut = Image.new('L', alpha.size, 255)
    cp = cut.load()
    for region in regions:
        for i in region:
            cp[i % w, i // w] = 0
    # 抗锯齿边缘比阈值内的像素多出约 1 像素。
    return ImageChops.multiply(alpha, cut.filter(ImageFilter.MinFilter(5)))


def color_field(img: Image.Image) -> Image.Image:
    """原图渐变铺满整张图：颜色按透明度加权模糊后归一化，加粗出来的像素也是原来的颜色。"""
    radius = 24
    alpha = img.getchannel('A')
    weight = alpha.filter(ImageFilter.GaussianBlur(radius)).load()
    channels = []
    for c in img.split()[:3]:
        blurred = ImageChops.multiply(c, alpha).filter(ImageFilter.GaussianBlur(radius)).load()
        out = Image.new('L', img.size)
        op = out.load()
        for y in range(img.height):
            for x in range(img.width):
                w = weight[x, y]
                op[x, y] = min(255, blurred[x, y] * 255 // w) if w else 0
        channels.append(out)
    return Image.merge('RGB', channels)


def shape(alpha: Image.Image, colors: Image.Image, size: int, out: int | None = None) -> Image.Image:
    """把线条加粗到 size 下约 MIN_STROKE 像素宽，上色，缩放到 out（默认等于 size）。"""
    grow = int(round((MIN_STROKE * alpha.width / size - SOURCE_STROKE) / 2))
    if grow > 0:
        alpha = alpha.filter(ImageFilter.MaxFilter(grow * 2 + 1))
    result = colors.convert('RGBA')
    result.putalpha(alpha)
    return result.resize((out or size, out or size), Image.LANCZOS)


def with_dot(icon: Image.Image) -> Image.Image:
    """右下角加橙色圆点，并在圆点周围挖出一圈透明，任何底色上都看得清。"""
    size = icon.width
    d = max(6, round(size * 0.4))
    gap = max(1, round(size * 0.06))
    x0 = y0 = size - d
    hole = Image.new('L', icon.size, 255)
    ImageDraw.Draw(hole).ellipse((x0 - gap, y0 - gap, size + gap, size + gap), fill=0)
    out = icon.copy()
    out.putalpha(ImageChops.multiply(icon.getchannel('A'), hole))
    # 圆点在 4 倍尺寸上画再缩小，边缘平滑。
    big = Image.new('RGBA', (size * 4, size * 4), (0, 0, 0, 0))
    ImageDraw.Draw(big).ellipse((x0 * 4, y0 * 4, size * 4 - 1, size * 4 - 1), fill=ATTENTION)
    return Image.alpha_composite(out, big.resize(icon.size, Image.LANCZOS))


def save_ico(frames: list[Image.Image], path: Path) -> None:
    frames[-1].save(path, format='ICO', sizes=[f.size for f in frames], append_images=frames[:-1])


def main() -> None:
    src = Image.open(SOURCE).convert('RGBA')
    colors = color_field(src)
    full = src.getchannel('A')
    parts = components(full)
    # 主体：重叠的三个泡泡（最大的连通区域）；小尺寸只画它。
    simple = without(full, parts[1:])
    # 右侧的星星与提醒圆点位置重叠，提醒版去掉它。
    right_star = [r for r in parts[1:] if min(i % full.width for i in r) > full.width * 0.7]
    full_for_dot = without(full, right_star)

    def frames(complete: Image.Image) -> list[Image.Image]:
        return [shape(simple if s <= SIMPLE_MAX else complete, colors, s) for s in SIZES]

    save_ico(frames(full), ASSETS / 'Poise.ico')
    save_ico([with_dot(f) for f in frames(full_for_dot)], ASSETS / 'PoiseAttention.ico')
    # 界面里显示约 16～20 DIP：完整图案，线条按 32px 加粗，存成 48px，高分屏上也清晰。
    shape(full, colors, 32, out=48).save(ASSETS / 'Poise.png')
    print('icons written to', ASSETS)


if __name__ == '__main__':
    main()
