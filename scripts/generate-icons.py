"""Regenerate platform icons from the shared, original slider mark. Requires Pillow."""
from pathlib import Path
import xml.etree.ElementTree as ET

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parent.parent
BRAND = ROOT / "assets/branding"
RES = ROOT / "android/app/src/main/res"
NS = {"svg": "http://www.w3.org/2000/svg"}


def main():
    source = ET.parse(BRAND / "lanmix.svg").getroot()
    background = source.find("svg:rect", NS).get("fill")
    lines, knobs = source.findall("svg:g", NS)
    accent = lines.get("stroke")
    scale = 2048 / 108
    image = Image.new("RGBA", (2048, 2048))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle((0, 0, 2047, 2047), radius=24 * scale, fill=background)
    line_paths, circle_paths = [], []
    for line in lines:
        x, y, end = (float(line.get(key)) for key in ("x1", "y1", "y2"))
        width = float(lines.get("stroke-width"))
        draw.line((x * scale, y * scale, x * scale, end * scale), fill=accent, width=round(width * scale))
        radius = width / 2
        for cy in (y, end):
            draw.ellipse(((x-radius)*scale, (cy-radius)*scale, (x+radius)*scale, (cy+radius)*scale), fill=accent)
        line_paths.append(f"M{x:g},{y:g}V{end:g}")
    for circle in knobs:
        x, y, radius = (float(circle.get(key)) for key in ("cx", "cy", "r"))
        stroke = float(knobs.get("stroke-width")) / 2
        for r, color in ((radius + stroke, accent), (radius - stroke, background)):
            draw.ellipse(((x-r)*scale, (y-r)*scale, (x+r)*scale, (y+r)*scale), fill=color)
        circle_paths.append(f"M{x-radius:g},{y:g}a{radius:g},{radius:g} 0,1 0,{2*radius:g},0a{radius:g},{radius:g} 0,1 0,{-2*radius:g},0")
    image.resize((512, 512), Image.Resampling.LANCZOS).save(BRAND / "lanmix.png", optimize=True)
    icon = image.resize((256, 256), Image.Resampling.LANCZOS)
    icon.save(BRAND / "lanmix.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
    # Android adaptive icons get separate background/foreground layers. The mark
    # stays inside the safe zone, and themed/notification icons use the same shape.
    header = '<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="108dp" android:height="108dp" android:viewportWidth="108" android:viewportHeight="108">\n'
    def vector(color, fill):
        return (header + f'    <path android:fillColor="#00000000" android:strokeColor="{color}" android:strokeWidth="5" android:strokeLineCap="round" android:pathData="{"".join(line_paths)}" />\n'
                + f'    <path android:fillColor="{fill}" android:strokeColor="{color}" android:strokeWidth="4" android:pathData="{"".join(circle_paths)}" />\n</vector>\n')
    (RES / "drawable/ic_lanmix.xml").write_text(vector(accent, background), encoding="utf-8")
    (RES / "drawable/ic_lanmix_mono.xml").write_text(vector("#FFFFFFFF", "#FFFFFFFF"), encoding="utf-8")
    (RES / "drawable/ic_lanmix_background.xml").write_text(
        '<shape xmlns:android="http://schemas.android.com/apk/res/android" android:shape="rectangle">\n'
        f'    <solid android:color="{background}" />\n</shape>\n', encoding="utf-8")
    # Notification graphics use a tighter viewport for legibility at 24 dp.
    notification = vector("#FFFFFFFF", "#FFFFFFFF").replace('width="108dp"', 'width="24dp"').replace('height="108dp"', 'height="24dp"')
    notification = notification.replace('viewportWidth="108" android:viewportHeight="108"', 'viewportWidth="60" android:viewportHeight="60"')
    # A group translates the same 108-unit mark into the cropped viewport.
    notification = notification.replace('\n    <path', '\n    <group android:translateX="-24" android:translateY="-24">\n    <path', 1).replace('</vector>', '    </group>\n</vector>')
    (RES / "drawable/ic_receiver.xml").write_text(notification, encoding="utf-8")
    print("Generated shared PNG, multi-size Windows ICO and Android icon layers")


if __name__ == "__main__":
    main()
