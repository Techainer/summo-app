#!/usr/bin/env python3
"""The picture inside the disk image, drawn rather than checked in as a mystery.

    python3 scripts/dmg-background.py

It is the first thing a Mac user sees, and until now it told them to open Terminal:

    Lần đầu mở, macOS sẽ chặn
    Bản này chưa mua chứng chỉ Apple. Mở Terminal, dán một dòng, xong vĩnh viễn:
    xattr -dr com.apple.quarantine /Applications/Summo.app

That is not what macOS does, and asking somebody to paste a command into a terminal to open a
meeting-notes app loses most of them at the first step. Asked directly, macOS says the signature is
fine:

    /Applications/Summo.app: valid on disk
    /Applications/Summo.app: satisfies its Designated Requirement
    syspolicy_check: Adhoc Signed App — Severity: Warning
                     Notary Ticket Missing

A *valid* signature that nobody vouches for gets the "cannot be opened because Apple cannot check
it" dialog and an **Open Anyway** button in System Settings. Only a *broken* signature gets "is
damaged", where that button never appears and a terminal is the only way through — which is what
v0.2.0 shipped, and which is where the wording came from and stayed after the cause was fixed.

So the panel says which two buttons to press. The command stays in the README for whoever prefers
it; it is not what a first-time user should be reading.

Both sizes are written: the `@2x` is what a Retina Mac actually shows, and it is the one that was
half a version behind last time somebody edited these by hand.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent.parent
OUT = HERE / "apps/desktop/src-tauri/dmg"

# The window is 660×400; `tauri.conf.json` puts the app at x=180 and Applications at x=480, both at
# y=170, so everything drawn here has to stay out of the band between them.
SIZE = (660, 400)

PAPER = (247, 246, 244)
INK = (23, 22, 26)
DIM = (86, 84, 94)
FAINT = (140, 137, 148)
ACCENT = (15, 115, 80)
LINE = (230, 227, 222)
CARD = (255, 255, 255)

FONTS = "/usr/share/fonts/truetype"


def font(name: str, size: int) -> ImageFont.FreeTypeFont:
    """A face that can draw Vietnamese. DejaVu has the diacritics; Liberation does not."""
    return ImageFont.truetype(f"{FONTS}/dejavu/{name}", size)


def rounded(draw: ImageDraw.ImageDraw, box, radius, fill, outline=None, width=1) -> None:
    draw.rounded_rectangle(box, radius=radius, fill=fill, outline=outline, width=width)


def compose(scale: int) -> Image.Image:
    image = Image.new("RGB", (SIZE[0] * scale, SIZE[1] * scale), PAPER)
    draw = ImageDraw.Draw(image)

    def at(x: float, y: float) -> tuple[float, float]:
        return (x * scale, y * scale)

    def text(x, y, string, face, fill, anchor="la") -> None:
        draw.text(at(x, y), string, font=face, fill=fill, anchor=anchor)

    title = font("DejaVuSans-Bold.ttf", 19 * scale)
    lead = font("DejaVuSans.ttf", 11 * scale)
    label = font("DejaVuSans-Bold.ttf", 11 * scale)
    body = font("DejaVuSans.ttf", 10 * scale)
    step = font("DejaVuSans-Bold.ttf", 10 * scale)

    text(330, 34, "Kéo Summo vào Applications", title, INK, anchor="ma")
    text(330, 62, "Biên bản họp chạy trên máy bạn. Không gửi audio đi đâu.", lead, DIM, anchor="ma")

    # The arrow between the two icons, which Finder draws at y=170.
    y = 168
    draw.line([at(258, y), at(400, y)], fill=(178, 175, 168), width=2 * scale)
    draw.polygon(
        [at(400, y - 6), at(416, y), at(400, y + 6)],
        fill=(178, 175, 168),
    )

    # What happens the first time, in the two presses it actually takes.
    top = 258
    rounded(draw, [at(40, top), at(620, 372)], 12 * scale, CARD, LINE, max(1, scale))

    text(60, top + 18, "Lần đầu mở, macOS sẽ hỏi — đừng bấm Move to Trash", label, ACCENT)
    text(
        60,
        top + 38,
        "Hộp thoại có hai nút: Move to Trash và Done. App không hỏng, chỉ là chưa mua chứng chỉ Apple.",
        body,
        DIM,
    )

    for index, (number, line) in enumerate(
        [
            ("1", "Bấm Done."),
            ("2", "Cài đặt hệ thống › Quyền riêng tư & Bảo mật › cuộn xuống, bấm Open Anyway."),
        ]
    ):
        row = top + 62 + index * 24
        draw.ellipse([at(60, row), at(74, row + 14)], fill=(235, 243, 239))
        text(67, row + 7, number, step, ACCENT, anchor="mm")
        text(84, row + 2, line, body, INK)

    text(60, top + 114, "Một lần thôi. Từ lần sau mở như mọi app khác.", body, FAINT)

    return image


for scale, name in ((1, "background.png"), (2, "background@2x.png")):
    picture = compose(scale)
    picture.save(OUT / name)
    print(f"{name} — {picture.size[0]}×{picture.size[1]}")
