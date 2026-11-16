# Wisp design and motion system

The whole interface is one file, [`src/ui.html`](../src/ui.html). Its first
`<style id="tokens">` block is the system; everything after it is components
built from those tokens. The rule that keeps it a system:

> **Components never use a raw color, size, or duration. They use a token.**

Users can override any token in Settings > Theme > Custom CSS, for example
`:root { --d-2: 400ms; --sp-2: 10px; }`.

## Tokens

| Group | Tokens | Notes |
| --- | --- | --- |
| Space | `--sp-1` … `--sp-6` (4, 8, 12, 16, 24, 32 px) | Padding and gaps. Nothing in between. |
| Shape | `--r-sm`, `--r-md`, `--r-lg`, `--r-full` | All derived from the user's roundness (`--radius`). |
| Type | `--text-xs` … `--text-xl`, `--font-ui`, `--font-mono` | All derived from the user's text size (`--font`). |
| Size | `--ctl` (buttons, inputs), `--row` (a sidebar tab), `--hotbar-h`, `--tabs-h`, `--side-w`, `--strip` | `top-compact` shrinks `--ctl`, `--hotbar-h`, `--tabs-h`. |
| Color | `--bg`, `--fg`, `--mute`, `--surface`, `--hover`, `--field`, `--line`, `--accent`, `--ws`, `--danger`, `--paint` | Semantic. Light values on `:root`, dark on `[data-theme=dark]`. |
| Elevation | `--shadow-1` (active tab), `--shadow-pop` (menus) | Two levels only. |
| Motion | `--d-1`, `--d-2`, `--d-3`, `--ease-out`, `--ease-in`, `--ease-move` | See below. |

### Color roles

- `--paint` is what the window is painted with: the user's gradient if they set
  one, otherwise the theme background tinted 9% with the active workspace's
  color (`--ws`). Switching workspace therefore re-tints the whole bar.
- `--surface` is anything raised off the paint: the active tab, cards.
- `--field` is anything you type into. `--hover` is pointer feedback.
- `--accent` marks focus, progress and "on" states. `--ws` marks the workspace.
- Text is `--fg`, or `--mute` for secondary text. With a custom background the
  theme (light or dark text) is picked from the background's brightness.
- Filled buttons are `--fg` on `--bg`, never white on `--accent`, so they stay
  readable whatever accent the user picks.

## Motion

Three durations and three curves. Pick by what is moving, not by taste.

| Token | Value | Use for |
| --- | --- | --- |
| `--d-1` | 120 ms | Feedback: hover, press, focus ring, and anything **leaving**. |
| `--d-2` | 200 ms | A part **entering**: a tab, a menu, the bar sliding in. |
| `--d-3` | 320 ms | A whole surface changing: a sheet opening, swapping workspace. |
| `--ease-out` | `cubic-bezier(.2, .8, .2, 1)` | Entering. Fast start, soft landing. |
| `--ease-in` | `cubic-bezier(.4, 0, 1, 1)` | Leaving. |
| `--ease-move` | `cubic-bezier(.3, .7, .2, 1)` | Moving between two places. |

Rules:

1. **Leaving is faster than entering.** Exits use `--d-1`; entrances `--d-2`.
2. **Move a short distance.** Things slide 6–28 px and fade; nothing crosses
   the window.
3. **Direction means something.** The bar slides in from the edge it lives on.
   Tabs slide in from the side the new workspace sits on in the workspace row.
   Menus grow from the point that opened them.
4. **Only new things animate.** Tabs are keyed by id, so a title update never
   replays an entrance.
5. **Reduced motion is honored.** With the system setting on, all three
   durations become 1 ms.

### Where motion is used

| Interaction | Animation |
| --- | --- |
| Collapsed bar revealed (mouse near the edge, or the edge strip clicked) | `bar-in-*`: slide 28 px from its edge + fade, `--d-2` `--ease-out` |
| Bar hiding | `bar-out`: slide 20 px back + fade, `--d-1` `--ease-in`; the page gets the space back only after it finishes |
| New tab | `tab-in`: drop 6 px + fade, `--d-2` |
| Switching workspace | `swap-next` / `swap-prev`: tab list slides 28 px, `--d-3` `--ease-move`; the bar tint cross-fades |
| Menu or workspace editor | `pop-in`: scale from .94 at its anchor, `--d-2` |
| Settings / Customize sheet | `sheet-in`: rise 10 px + fade, `--d-3` |
| Page loading | spinner in the tab, sweep under the hotbar |
| Hover, press | background `--d-1`; buttons press to 95% |

### One constraint to know

Tab pages are native views stacked **above** the interface, so the interface
cannot draw over a page. That is why a revealed bar pushes the page aside
(instantly) and then animates into the space it made, rather than floating over
the page, and why a menu that would overlap the page hides the page while open.

## Layout

The window is a grid of three parts: **hotbar**, **sidebar** (a column, or the
tab row in top-bar layouts) and **page**. The interface reports the gap it
leaves for the page and the window places the active tab exactly there, so
layout is decided in CSS only.

Items (address bar, buttons, workspaces, spacers) live in three areas the user
can rearrange in Customize mode: `hotbar`, `before_tabs`, `after_tabs`. The tab
list itself is fixed between the last two.

## Adding a component

1. Build it from tokens. If a value is missing, add a token, not a number.
2. Give it hover and focus-visible states (`--hover`, the accent outline).
3. If it appears or disappears, use an existing keyframe and the duration rule
   above before inventing a new animation.
4. Check it in both themes and with a custom background preset.
