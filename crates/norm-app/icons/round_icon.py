import sys
import os

try:
    from PIL import Image, ImageDraw
except ImportError:
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "Pillow"])
    from PIL import Image, ImageDraw

def add_corners(im, rad):
    circle = Image.new('L', (rad * 2, rad * 2), 0)
    draw = ImageDraw.Draw(circle)
    draw.ellipse((0, 0, rad * 2 - 1, rad * 2 - 1), fill=255)
    alpha = Image.new('L', im.size, 255)
    w, h = im.size
    alpha.paste(circle.crop((0, 0, rad, rad)), (0, 0))
    alpha.paste(circle.crop((0, rad, rad, rad * 2)), (0, h - rad))
    alpha.paste(circle.crop((rad, 0, rad * 2, rad)), (w - rad, 0))
    alpha.paste(circle.crop((rad, rad, rad * 2, rad * 2)), (w - rad, h - rad))
    im.putalpha(alpha)
    return im

icon_path = "/Users/adchanapong/Desktop/norm_note/crates/norm-app/icons/app-icon.png"
if not os.path.exists(icon_path):
    print("Icon not found at", icon_path)
    sys.exit(1)
    
im = Image.open(icon_path).convert("RGBA")
# For macOS squircle, radius is roughly 22.5% of width. Or 225px for 1024x1024.
radius = int(im.width * 0.225)
im = add_corners(im, radius)
im.save("/Users/adchanapong/Desktop/norm_note/crates/norm-app/icons/app-icon.png")
print("Rounded icon saved.")
