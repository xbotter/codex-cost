#!/usr/bin/env python3

from __future__ import annotations

import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "artifacts" / "branding" / "codex-cost-logo-mark.png"
BRANDING_DIR = ROOT / "artifacts" / "branding"
ICONS_DIR = ROOT / "src-tauri" / "icons"


def resized(image: Image.Image, size: int) -> Image.Image:
    return image.resize((size, size), Image.Resampling.LANCZOS)


def save_png_set(image: Image.Image) -> None:
    png_targets = {
        "32x32.png": 32,
        "64x64.png": 64,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 1024,
        "StoreLogo.png": 50,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
    }
    for name, size in png_targets.items():
        resized(image, size).save(ICONS_DIR / name)


def save_ios_set(image: Image.Image) -> None:
    ios_targets = {
        "AppIcon-20x20@1x.png": 20,
        "AppIcon-20x20@2x-1.png": 40,
        "AppIcon-20x20@2x.png": 40,
        "AppIcon-20x20@3x.png": 60,
        "AppIcon-29x29@1x.png": 29,
        "AppIcon-29x29@2x-1.png": 58,
        "AppIcon-29x29@2x.png": 58,
        "AppIcon-29x29@3x.png": 87,
        "AppIcon-40x40@1x.png": 40,
        "AppIcon-40x40@2x-1.png": 80,
        "AppIcon-40x40@2x.png": 80,
        "AppIcon-40x40@3x.png": 120,
        "AppIcon-60x60@2x.png": 120,
        "AppIcon-60x60@3x.png": 180,
        "AppIcon-76x76@1x.png": 76,
        "AppIcon-76x76@2x.png": 152,
        "AppIcon-83.5x83.5@2x.png": 167,
        "AppIcon-512@2x.png": 1024,
    }
    for name, size in ios_targets.items():
        resized(image, size).save(ICONS_DIR / "ios" / name)


def save_android_set(image: Image.Image) -> None:
    android_targets = {
        "mipmap-mdpi": 48,
        "mipmap-hdpi": 72,
        "mipmap-xhdpi": 96,
        "mipmap-xxhdpi": 144,
        "mipmap-xxxhdpi": 192,
    }
    for folder, size in android_targets.items():
        for name in ("ic_launcher.png", "ic_launcher_round.png", "ic_launcher_foreground.png"):
            resized(image, size).save(ICONS_DIR / "android" / folder / name)


def save_ico(image: Image.Image) -> None:
    image.save(
        ICONS_DIR / "icon.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )


def save_icns(image: Image.Image) -> None:
    with tempfile.TemporaryDirectory() as temp_dir:
        iconset = Path(temp_dir) / "codex-cost.iconset"
        iconset.mkdir(parents=True, exist_ok=True)
        sizes = {
            "icon_16x16.png": 16,
            "icon_16x16@2x.png": 32,
            "icon_32x32.png": 32,
            "icon_32x32@2x.png": 64,
            "icon_128x128.png": 128,
            "icon_128x128@2x.png": 256,
            "icon_256x256.png": 256,
            "icon_256x256@2x.png": 512,
            "icon_512x512.png": 512,
            "icon_512x512@2x.png": 1024,
        }
        for name, size in sizes.items():
            resized(image, size).save(iconset / name)
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(ICONS_DIR / "icon.icns")],
            check=True,
        )


def main() -> None:
    image = Image.open(SOURCE).convert("RGBA")
    shutil.copy2(SOURCE, BRANDING_DIR / "codex-cost-logo-preview.png")
    save_png_set(image)
    save_ios_set(image)
    save_android_set(image)
    save_ico(image)
    save_icns(image)


if __name__ == "__main__":
    main()
