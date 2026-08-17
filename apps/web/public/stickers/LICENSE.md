# Stickers

The animations in this folder are **Noto Emoji Animation** by Google, used under
**CC BY 4.0** — https://creativecommons.org/licenses/by/4.0/

Source: https://googlefonts.github.io/noto-emoji-animation/ · https://github.com/googlefonts/noto-emoji

| File | Emoji | Codepoint |
|---|---|---|
| `sprout.json` | 🌱 seedling | `1f331` |
| `party.json` | 🎉 party popper | `1f389` |
| `wave.json` | 👋 waving hand | `1f44b` |
| `coffee.json` | ☕ hot beverage | `2615` |
| `magnifier.json` | 🔎 magnifying glass tilted right | `1f50e` |
| `robot.json` | 🤖 robot | `1f916` |
| `pencil.json` | ✍️ writing hand | `270d_fe0f` |

They are served as static files rather than imported, so they are fetched only when a sticker is
actually on screen and never enter a JavaScript chunk. `components/ui/Sticker.tsx` draws its own SVG
underneath each one — that is what renders while the JSON is in flight, when the fetch fails, and
under `prefers-reduced-motion`.
