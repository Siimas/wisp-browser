# The Wisp brand

**Wisp is a light, quiet browser that frames the web and then gets out of the way.**

A wisp is a thin trail of light or smoke: there, unmistakable, and nearly
weightless. Everything below follows from that.

## How it should feel

| Word | Means | Rules out |
| --- | --- | --- |
| **Quiet** | Medium contrast at rest; one accent; emphasis from spacing. | Badges, counters, borders around every control, loud color. |
| **Light** | Thin lines, short motion, little on screen, little in memory. | Heavy chrome, decorative animation, anything that runs while idle. |
| **Deep** | Rich, slightly tinted paint with a fine grain; the page sits on it like a card. | Flat gray "default app" surfaces, pure black, pure white. |
| **Exact** | Consistent radii, hairlines that always show, text that always reads. | One-off shadows, near-misses in alignment, "good enough at one size". |

## Palette

The mark is a white crescent on a rounded square that sweeps from **Wisp blue**
to **Wisp violet** (`assets/make_icon.py`). The interface uses the same sweep,
but softly: the window's default paint is a 160° gradient from the workspace
color towards its neighbor 34° further round the color wheel. For the default
workspace that is the icon's blue to violet. Violet never stands alone; it is
where the sweep ends.

| Name | Hex | Use |
| --- | --- | --- |
| Wisp blue | `#6c8cff` | The default accent and workspace color; focus rings; "on". |
| Wisp violet | `#a86cff` | The far end of the sweep and of the icon. Never a fill on its own. |
| Ink (dark background) | `#121319` | Dark theme `--bg`: a cool blue-black. |
| Ink text | `#eceef3` / `#9b9eab` | Dark `--fg` / `--mute`. |
| Haze (light background) | `#eef0f5` | Light theme `--bg`: a cool near-white. |
| Haze text | `#1b1c22` / `#5c5f6b` | Light `--fg` / `--mute`. |
| Moon | `#ffffff` | The crescent, and the page card. |

The neutrals are **cool**, leaning a few degrees towards the blue of the mark,
so the sweep reads as light falling on them rather than as a color pasted on.

### Directions considered

Three neutral families were tried in the real window with the signature paint
and grain:

- **A. Cool ink** (`#121319` / `#eef0f5`), chosen. Crisp; the blue-violet sweep
  does the talking; dark reads as night sky rather than as gray.
- **B. Violet dusk** (`#17131f` / `#f0edf6`). Warmer and more "purple app".
  Pretty, but violet everywhere leaves the sweep nowhere to go and fights the
  warm presets (Ember, Linen).
- **C. Warm graphite** (`#171615` / `#f2f1ee`). Calm, but the blue mark looks
  pasted onto it and the dark theme turns brown-gray next to a white page.

## Texture: the grain

Wisp's paint carries a fine, monochrome grain: one grain per device pixel,
mid-gray on average, blended with `overlay`. It makes the paint read as a
material instead of a screen fill and dithers gradient banding away. It is
the texture every preset uses. Details and cost in
[`DESIGN.md`](DESIGN.md#texture).

## Type and icons

- **Type** is the system font (`system-ui`), weights 400 and 500 only, sizes
  from the user's text size. No bold, no display fonts in the interface.
- **Icons** are line icons on a 16 px grid, 1.5 px stroke, round caps, in
  `currentColor`. The crescent is the only filled shape and belongs to the mark.
- **No emoji** anywhere in the interface.
- **The name** is written "Wisp", capital W, never all caps.

## Do and don't

| Do | Don't |
| --- | --- |
| Let the paint carry color; keep controls `--mute` until they matter. | Color buttons, borders or text with the brand gradient. |
| Use the blue-violet sweep diagonally, soft, low saturation. | Use violet as a solid fill or put the gradient behind text. |
| Keep grain fine and quiet: you notice its absence more than its presence. | Make grain visible at arm's length, colored, or animated. |
| Use white for the crescent and the page. | Put the crescent on anything but the sweep or plain ink/haze. |
| Raise only the current thing (active tab, focused field). | Box every control, or add a shadow that is not an elevation level. |
| Name themes after light and air (Mist, Daybreak, Dusk). | Name them after brands, or after effects ("Neon", "Glitch"). |
