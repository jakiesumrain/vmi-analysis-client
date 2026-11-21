"""Generate the VMI Analysis Client icon set.

Run from the repository root with Pillow available:

    python src-tauri/icons/generate-icons.py

The mark is a dark rounded square with a white "V" and a blue tunnel dot,
matching the desktop app's neutral dark palette.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

OUT = Path(__file__).resolve().parent
SIZE = 1024
RADIUS = int(SIZE * 0.22)
BACKGROUND = (15, 23, 42)  # slate-900
GLYPH = (248, 250, 252)  # slate-50
ACCENT = (59, 130, 246)  # blue-500


def load_font(size: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    candidates = [
        Path("C:/Windows/Fonts/segoeuib.ttf"),
        Path("C:/Windows/Fonts/arialbd.ttf"),
        Path("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
    ]
    for path in candidates:
        if path.exists():
            return ImageFont.truetype(str(path), size)
    return ImageFont.load_default()


def draw_mark() -> Image.Image:
    image = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle((0, 0, SIZE - 1, SIZE - 1), radius=RADIUS, fill=BACKGROUND)

    font = load_font(int(SIZE * 0.62))
    text = "V"
    box = draw.textbbox((0, 0), text, font=font)
    width = box[2] - box[0]
    height = box[3] - box[1]
    draw.text(
        ((SIZE - width) / 2 - box[0], (SIZE - height) / 2 - box[1] - SIZE * 0.04),
        text,
        font=font,
        fill=GLYPH,
    )

    dot = int(SIZE * 0.075)
    cx, cy = int(SIZE * 0.5), int(SIZE * 0.78)
    draw.ellipse((cx - dot, cy - dot, cx + dot, cy + dot), fill=ACCENT)
    return image


def main() -> None:
    mark = draw_mark()
    sizes = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }
    for name, size in sizes.items():
        mark.resize((size, size), Image.LANCZOS).save(OUT / name)

    mark.resize((256, 256), Image.LANCZOS).save(
        OUT / "icon.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print(f"wrote {len(sizes) + 1} icon files to {OUT}")


if __name__ == "__main__":
    main()
