"""由 icon/foam.png（512×512，透明底）生成安装程序向导的图片。

    python scripts/make-installer-images.py

输出到 installer/images/（透明底，浅色和深色外观都能用；每种图片多个尺寸，对应不同的显示缩放）：
- wizard-<缩放>.png  欢迎页与完成页左侧的大图：标志居中偏上
- small-<缩放>.png   其他页面右上角的小图：标志
依赖 Pillow。
"""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'icon' / 'foam.png'
OUTPUT = ROOT / 'installer' / 'images'

# 100% 缩放下的尺寸（Inno Setup 向导图片的标准尺寸）。
WIZARD = (164, 314)
SMALL = (55, 55)
SCALES = [100, 125, 150, 200, 250]


def logo(size: int) -> Image.Image:
    return Image.open(SOURCE).convert('RGBA').resize((size, size), Image.LANCZOS)


def wizard(scale: int) -> Image.Image:
    w, h = (round(v * scale / 100) for v in WIZARD)
    canvas = Image.new('RGBA', (w, h), (0, 0, 0, 0))
    mark = logo(round(w * 0.62))
    canvas.alpha_composite(mark, ((w - mark.width) // 2, round(h * 0.36 - mark.height / 2)))
    return canvas


def small(scale: int) -> Image.Image:
    w, h = (round(v * scale / 100) for v in SMALL)
    canvas = Image.new('RGBA', (w, h), (0, 0, 0, 0))
    mark = logo(round(w * 0.9))
    canvas.alpha_composite(mark, ((w - mark.width) // 2, (h - mark.height) // 2))
    return canvas


def main() -> None:
    OUTPUT.mkdir(parents=True, exist_ok=True)
    for scale in SCALES:
        wizard(scale).save(OUTPUT / f'wizard-{scale}.png', optimize=True)
        small(scale).save(OUTPUT / f'small-{scale}.png', optimize=True)
    print(f'完成：{OUTPUT}')


if __name__ == '__main__':
    main()
