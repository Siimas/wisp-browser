# Wisp design and motion system

The whole interface is one file, [`src/ui.html`](../src/ui.html). Its first
`<style id="tokens">` block is the system; everything after it is components
built from those tokens. The rule that keeps it a system:

> **Components never use a raw color, size, or duration. They use a token.**

Users can override any token in Settings > Theme > Custom CSS, for example
`:root { --d-3: 450ms; --gap: 10px; }`.

## The look in one paragraph

Quiet and layered. The window is painted in one deep, slightly tinted color.
The page sits on it as a rounded **card** with a hairline edge and a soft
shadow. The bars have no borders or boxes of their own: controls rest directly
on the paint, and only the thing you are on (the active tab, the focused
address bar) is raised. Text is medium contrast until it matters. Nothing
shouts; emphasis comes from spacing and from one accent color.

## Tokens

| Group | Tokens | Notes |
| --- | --- | --- |
| Space | `--sp-1` … `--sp-6` (4, 8, 12, 16, 24, 32 px), `--gap` (8 px) | `--gap` is the one distance between window edge, page card and bars. |
| Shape | `--r-sm`, `--r-md`, `--r-lg`, `--r-full`, `--page-r` | All derived from the user's roundness (`--radius`). |
| Type | `--text-xs` … `--text-xl`, `--font-ui`, `--font-mono` | All derived from the user's text size (`--font`). Weights: 400 body, 500 emphasis. No bold. |
| Size | `--ctl` 28 (buttons, inputs), `--row` 36 (a sidebar tab), `--hotbar-h` 40, `--tabs-h` 40, `--side-w` 220 | `top-compact` shrinks `--ctl`, `--hotbar-h`, `--tabs-h`. |
| Color | `--bg`, `--fg`, `--mute`, `--surface`, `--hover`, `--field`, `--line`, `--accent`, `--ws`, `--danger`, `--paint` | Semantic. Light values on `:root`, dark on `[data-theme=dark]`. |
| Elevation | `--shadow-0`, `--shadow-1`, `--shadow-card`, `--shadow-pop` | Four levels, see Elevation below. |
| Motion | `--d-1`, `--d-2`, `--d-3`, `--ease-out`, `--ease-in`, `--ease-move` | See below. |

### Color roles

- `--paint` is what the window is painted with: the user's gradient if they set
  one, otherwise the theme background tinted 9% with the active workspace's
  color (`--ws`). Switching workspace re-tints the whole window.
- `--surface` is anything raised off the paint: the active tab, cards, the
  focused address bar.
- `--field` is anything you type into at rest. `--hover` is pointer feedback.
- `--accent` marks focus, progress and "on" states. `--ws` marks the workspace.
- Text is `--fg`, or `--mute` for anything secondary (inactive tabs, icons at
  rest, the path part of an address). With a custom background, light or dark
  text is picked from the background's brightness.
- Filled buttons are `--fg` on `--bg`, never white on `--accent`, so they stay
  readable whatever accent the user picks.

### Elevation

Shadows say how far something stands off the paint, and give it an edge where
color alone would not. Four levels, used by role:

| Token | Level | Used for |
| --- | --- | --- |
| `--shadow-0` | Recessed | Things you type into at rest (the address bar, the editor canvas): an inner hairline. |
| `--shadow-1` | Raised | The current thing and anything pressable that needs an edge: the active tab, the active workspace, buttons, cards. |
| `--shadow-card` | Card | The page. |
| `--shadow-pop` | Floating | Menus and the revealed bar. |

Every level carries a hairline ring so the edge survives on any background. On
dark paint a drop shadow alone is invisible, so the dark values add a lit top
edge (an inset highlight): that, not the shadow, is what reads as "raised".
Never add a one-off `box-shadow`; pick a level.

### Glass

Floating things are glass; docked things are solid. That is the whole rule.

- **Glass**: the revealed (floating) bar and every menu or popover. Their
  background is the theme color at 40–64% opacity over a blur of what lies
  behind, with a hairline ring and a one-pixel top highlight.
- **Solid**: the docked sidebar, the hotbar, the page card, sheets and cards.

Menus blur with CSS `backdrop-filter` (they sit over the interface itself). The
floating bar sits over the page, which is a separate native view, so on macOS a
native frosted pane (`native::Glass`) is placed directly beneath it. Windows has
no such pane yet and keeps the floating bar opaque.

### Icons

Line icons only: 16 px grid, 1.5 px stroke, round caps and joins, `currentColor`.
They live in the `ICON` map in `ui.html`. **No emoji anywhere in the
interface**: emoji bring their own colors and weights and break the calm.
Workspace icons are picked from the same set and take the workspace's color.

### Details that carry the feel

- **The address bar** shows the site's own domain in `--fg` at weight 500 and
  the rest (subdomain, path) in `--mute`; `https://` and any `user:password@`
  are dropped. Focused, it shows the full address to edit, lifts to
  `--surface` and gains an accent ring.
- **Long tab titles fade out** (a mask) instead of ending in "…".
- **Close buttons appear on hover only.**
- **macOS window buttons** are moved to sit on the center line of the first
  row of controls, at `--gap` + 5 px from the left edge.

### The theme editor

Settings > Theme is a small instrument rather than a form, modelled on a color
wheel laid flat:

- **Canvas**: each color is a dot. Its direction from the center is the hue,
  its distance the vividness. The first dot is larger; two or more dots make a
  gradient that runs from the first towards the last. Buttons on the canvas:
  match system / light / dark above; add a color, remove one, and a harmony
  toggle (free, neighbors, opposite, triad) below.
- **Swatch row**: three pages of ready colors applied to the selected dot.
- **Wave slider**: how strongly the colors cover the theme background.
- **Dial**: texture strength; turning it up from zero switches grain on.

Text flips between light and dark automatically if the painted result would
make the chosen one unreadable.

## Motion

Three durations and three curves. Pick by what is moving, not by taste.

| Token | Value | Use for |
| --- | --- | --- |
| `--d-1` | 130 ms | Feedback (hover, press, focus) and anything **leaving**. |
| `--d-2` | 220 ms | A part **entering**: a tab, a menu. |
| `--d-3` | 320 ms | A whole surface: the bar, the page card resizing, a sheet, a workspace swap. |
| `--ease-out` | `cubic-bezier(.22, 1, .36, 1)` | Entering: quick start, long soft landing. |
| `--ease-in` | `cubic-bezier(.5, 0, .9, .4)` | Leaving. |
| `--ease-move` | `cubic-bezier(.3, .7, .2, 1)` | Moving between two places. |

Rules:

1. **Leaving is faster than entering.** Exits use `--d-1`; entrances `--d-2`
   or `--d-3`. Things get out of the way quickly and arrive gently.
2. **Move a short distance.** Things travel 12–36 px and fade; nothing crosses
   the window.
3. **Direction means something.** The bar comes from the edge it lives on.
   Tabs slide in from the side the new workspace sits on. Menus grow from the
   point that opened them.
4. **Neighbors make room.** A new tab grows from zero height so the list below
   glides down; a closed tab folds away so the list glides up. Nothing jumps.
5. **Only new things animate.** Tabs are keyed by id, so a title update never
   replays an entrance.
6. **Pressing has weight.** Buttons press to 94% and tabs to 98.5% on
   mouse-down.
7. **Reduced motion is honored.** With the system setting on, all three
   durations become 1 ms.

### Where motion is used

| Interaction | Animation |
| --- | --- |
| Collapsed bar revealed (cursor within 10 px of its window edge) | The bar floats in over the page as a panel: slides 36 px from its edge, scales from 98.5%, fades in. `--d-3` `--ease-out` |
| Revealed bar let go (cursor away for 300 ms) | Slides back 22 px and fades. `--d-1` `--ease-in` |
| Bar collapsed or pinned open, layout changed | The page card **glides** to its new size over `--d-3` with the same soft landing (driven natively, `PAGE_GLIDE` in `main.rs`) while the bar slides in over `--d-3` |
| New tab / closed tab | Grows from / folds to zero height with a fade. `--d-2` in, `--d-1` out |
| Switching workspace | Tab list slides 32 px from that workspace's side, `--d-3`; the window tint changes with it |
| Menu or workspace editor | Scales from 94% at its anchor, `--d-2` |
| Settings / Customize sheet | Rises 10 px and fades in, `--d-3` |
| Address bar focus | Lifts to `--surface`, accent ring grows, `--d-2` |
| Page loading | Spinner in the tab, sweep under the hotbar |
| Hover, press | Background `--d-1`; press scale as above |

## Structure

The window stacks three kinds of native view, bottom to top:

1. **The interface**, filling the whole window: hotbar, sidebar, settings.
2. **One view per tab.** The interface measures the gap it leaves for the page
   (the `#page` card) and reports it; the active tab is placed exactly there
   and clipped to the card's corner radius.
3. **The floater**: a second copy of the interface, shown only while a
   collapsed bar is revealed, so the bar can float over the page. It is
   transparent except for the panel it draws.

Consequences worth knowing when designing:

- Layout is decided in CSS only. Rust knows no bar sizes; it follows the
  reported gap.
- The main interface cannot draw over a page. Menus opened from the sidebar
  are kept inside the sidebar (narrowing to fit). A menu that still has to
  overlap the page hides the page while it is open.
- Page corners are rounded on macOS; on Windows the card is square for now.

### Layouts

- **Sidebar right / left**: hotbar across the top (address bar centered,
  navigation left, tools right), then the page card beside the sidebar. The
  sidebar reads top to bottom: workspace name, a hairline, New tab, tabs, and
  the workspace switcher at the bottom.
- **Top, relaxed / compact**: tab row, hotbar, page card.

Items (address bar, buttons, workspace switcher, spacers) live in three areas
the user can rearrange in Customize mode: `hotbar`, `before_tabs`,
`after_tabs`. The tab list itself is fixed between the last two.

## Adding a component

1. Build it from tokens. If a value is missing, add a token, not a number.
2. Rest quietly: `--mute` text or icon on the paint, no border. Raise it
   (`--surface`) only when it is the current thing.
3. Give it hover and focus-visible states (`--hover`, the accent outline) and
   a press response.
4. If it appears or disappears, use an existing keyframe and the duration
   rules above before inventing a new animation, and make its neighbors make
   room.
5. Use a line icon from `ICON`; draw a new one on the same grid if needed.
6. Check it for real: in a debug build on macOS, `WISP_SHOT=/tmp/wisp.png`
   saves a picture of the window after six seconds and quits (`WISP_PEEK=1`
   reveals a collapsed bar first, `WISP_ACT=settings` runs an action,
   `WISP_JS=...` runs a script in the interface). Look at it in both themes
   and with a preset background.
