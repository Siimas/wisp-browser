# Wisp design and motion system

The whole interface is one file, [`src/ui.html`](../src/ui.html). Its first
`<style id="tokens">` block is the system; everything after it is components
built from those tokens. The rule that keeps it a system:

> **Components never use a raw color, size, or duration. They use a token.**

Users can override any token in Settings > Appearance > Custom CSS, for example
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
| Space | `--sp-0` (2 px), `--sp-1` … `--sp-6` (4, 8, 12, 16, 24, 32 px), `--gap` (8 px), `--fade` (18 px) | `--gap` is the one distance between window edge, page card and bars. `--sp-0` is only for neighbors in one tight list. `--fade` is how far a long title fades out. |
| Shape | `--r-sm`, `--r-md`, `--r-lg`, `--r-full`, `--page-r` | All derived from the user's roundness (`--radius`). |
| Type | `--text-xs` … `--text-xl`, `--font-ui`, `--font-mono` | All derived from the user's text size (`--font`). Weights: 400 body, 500 emphasis. No bold. |
| Size | `--ctl` 28 (buttons, inputs), `--row` 36 (a sidebar tab), `--hotbar-h` 40, `--tabs-h` 40, `--side-w` 220, `--lights` 82, `--strip` 28 | `top-compact` shrinks `--ctl`, `--hotbar-h`, `--tabs-h`. `--lights` and `--strip` are the room kept for the window buttons (on Windows: three 46 px `--cap-w` buttons plus `--gap`, and a 32 px strip). |
| Color | `--bg`, `--fg`, `--mute`, `--surface`, `--hover`, `--field`, `--line`, `--accent`, `--ws`, `--danger`, `--paint` | Semantic. Light values on `:root`, dark on `[data-theme=dark]`. |
| Elevation | `--shadow-0`, `--shadow-1`, `--shadow-card`, `--shadow-pop` | Four levels, see Elevation below. |
| Motion | `--d-1`, `--d-2`, `--d-3`, `--ease-out`, `--ease-in`, `--ease-move` | See below. |

### Color roles

- `--paint` is what the window is painted with: the user's gradient if they set
  one, otherwise `--signature`, the brand sweep (see [BRAND.md](BRAND.md)): a
  160° gradient from the workspace color (`--ws`) towards its neighbor 34°
  further round the wheel, laid over the theme background at `--sig-a` /
  `--sig-b` (9/13% light, 13/21% dark, since color reads weaker on
  near-black). Switching workspace re-tints the whole window. Where the engine
  lacks relative colors it falls back to a flat 9% tint.
- `--tint` is the paint as one color (the average of its ends), for glass.
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
  background is the theme's own color (`--tint`, menus use `--bg`) at 40%
  (light) or 80% (dark: a thinner dark glass turns gray over a white page)
  over a blur of what lies behind, with a hairline ring and a one-pixel top
  highlight.
- **Solid**: the docked sidebar, the hotbar, the page card, sheets and cards.

Menus blur with CSS `backdrop-filter` (they sit over the interface itself). The
floating bar sits over the page, which is a separate native view, so on macOS a
native frosted pane (`native::Glass`) is placed directly beneath it. Windows has
no such pane yet and keeps the floating bar opaque.

### Low-power mode

Settings > Behavior > Low-power mode: Automatic (the default), On, Off. It keeps
the layout, colors, texture and shadows, and drops what costs work while you use
the browser:

- **No glass.** The floating bar is the paint, solid, with no native frosted pane
  under it; menus and suggestions are `--bg`, solid, with no `backdrop-filter`
  (`[data-solid]`). The system's "Reduce transparency" setting does the same on
  its own, whatever the mode.
- **No motion.** `[data-low]` sets `--d-1` … `--d-3` to 1 ms, the same path as
  reduced motion, and the page card jumps to a new size instead of gliding.

Automatic turns it on while the system is saving power (macOS Low Power Mode,
Windows battery saver), on battery at about 20% or less, or on a computer with
4 GB of memory or less or two cores or fewer. It is decided again when the power
source or battery level changes (macOS tells the app) and when the window is
focused (for the switches that send nothing).

What was measured and left on, because at rest it costs nothing: the gradient
paint, the shadows, and the texture. The texture used to cost about 50 MB (a
window-sized `position: fixed` layer under a blend mode); it is now
`position: absolute`, which draws the same pixels without the extra layer.

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
- **Pinned tabs are tiles**: icon only, resting on `--field`, in a grid above
  "New tab" in a sidebar and at the start of the row in a top bar. They have
  no close button; the close-tab hotkey only steps off them.
- **macOS window buttons** are moved to sit on the center line of the first
  row of controls, at `--gap` + 5 px from the left edge. While a tucked-away
  top bar floats in, they move onto its tab row the same way.
- **Windows window buttons** are drawn by the interface (`#caption`), as the
  system draws them: 46 px wide, the full height of the first row, flush with
  the window's top-right corner, glyphs from Segoe Fluent Icons (Segoe MDL2
  Assets on Windows 10), close turning #c42b1c on hover. The first row is the
  title bar; there is no other.

### The theme editor

Settings > Appearance > Theme is a small instrument rather than a form, modelled on a color
wheel laid flat:

- **Canvas**: each color is a dot. Its direction from the center is the hue,
  its distance the vividness. The first dot is larger; two or more dots make a
  gradient that runs from the first towards the last. Buttons on the canvas:
  match system / light / dark above; add a color, remove one, and a harmony
  toggle (free, neighbors, opposite, triad) below.
- **Swatch row**: three pages of ready colors applied to the selected dot.
- **Wave slider**: how strongly the colors cover the theme background.
- **Dial**: texture strength; turning it up from zero switches grain on.
- **Presets**: a row of swatches under the editor, each painted exactly as the
  theme would be (paint, strength, grain) with its accent as a dot, and named
  beneath. The one in use is ringed and its name is in `--fg`. Picking one sets
  the colors, angle, strength, texture and accent; light or dark stays the
  user's. The curated set and its intent is in `PRESETS` in `ui.html`.

### Texture

Textures are one layer (`body::before`, `position: absolute`, not `fixed`:
fixed under a blend mode cost about 50 MB) over the paint: painted once,
never animated, no layout. Grain is the default and the one presets use.

**Grain** is a 128 × 128 tile generated at start-up (`grain()` in `ui.html`):
seeded white noise minus its 3 × 3 local average, gray with a mean of exactly
50%, blended with `overlay`. Why each part:

- **One grain per device pixel.** The tile is shown at `128 / devicePixelRatio`
  CSS pixels with `image-rendering: pixelated`, and regenerated if the density
  changes, so grain is equally fine on a Retina and a 1× screen instead of
  being scaled into soft blotches. Coarse grain reads as noise; pixel-fine
  grain reads as material.
- **High-passed.** Subtracting the local average removes the low frequencies
  that make plain noise look blotchy and makes the 128 px repeat invisible:
  close to the blue-noise character of a pre-rendered tile, without shipping
  one.
- **Monochrome, mean gray, overlay.** It changes brightness only, never hue,
  and averages to zero, so turning it up does not darken or tint the paint
  (measured: the mean color of a captured region moves by 0.1 / 255). Overlay
  is strongest in the mid-tones and fades towards black and white, which is
  where banding shows and where grain would look dirty, respectively.
- **It dithers banding.** Down a dark sidebar gradient at 2×, without grain
  the color steps in flat runs of up to 22 px; with grain at 40% no run is
  longer than 3 px.

**Cost**, measured in the debug build on macOS: generating the tile takes
about 12 ms once at start-up (2.5 ms on a later density change) and it is a
35 KB image. Drawing it is one tiled, static layer the compositor keeps; it
never repaints unless the window resizes. The previous SVG turbulence filter
was rasterized by the engine at an arbitrary scale and blurred on 1× screens.

Considered and not taken: a pre-rendered blue-noise PNG (same look, but a
binary asset in the repository and fixed to one density); grain strength
varying across the gradient (overlay already does this by tone); a vignette
or light falloff (it would compete with the user's gradient angle and darken
the corner where the macOS window buttons sit).

Dots, lines and grid stay available as plain CSS patterns in `--fg`.

Text flips between light and dark automatically if the painted result would
make the chosen one unreadable.

## Spacing and alignment

Measured, not eyeballed: check a change with `WISP_SHOT` captures and with
`getBoundingClientRect()` numbers read through `WISP_JS`. The rules:

1. **Only tokens.** Every gap, padding, margin, inset and control height is a
   space token, a size token, or arithmetic on them
   (`calc(var(--ctl) + 2 * var(--sp-2))`). Raw numbers are left only for a
   drawing's own geometry (the layout pictures, the dial, the color dots, a
   16 px icon) and for the widths of particular fields and popovers.
2. **One value per pattern.**

   | Pattern | Value |
   | --- | --- |
   | Window edge, page card, bars | `--gap` |
   | Items in a bar (hotbar, tab row, find bar, workspace switcher) | `--sp-1` |
   | Tab rows in a sidebar; tabs in a tab row | `--sp-0` |
   | Pinned tiles, in a grid or a row | `--sp-1` |
   | Icon to its label (tabs, menus, lists, fields) | `--sp-2` |
   | Text to the edge of its row or card | `--sp-3` (rows in cards, fields), `--sp-2` (tabs, list rows, menu items) |
   | Between a row's text and its control | `--sp-4` |
   | Parts of one instrument (the theme editor) | `--sp-2`, the same as its padding |

3. **Columns line up.** Icons, text and controls in a row share a center line.
   Down a list, icons sit in one 16 px column (a dot takes the same 16 px), so
   titles share a left edge. A section header (`.sheet h2`) is inset `--sp-3`
   so it sits on the text of the rows below it. The address bar and the
   suggestions under it share the icon and text columns: icon at `--sp-3`,
   text `--sp-2` after it. A menu's labels sit on its items' text.
4. **One height per context.** Controls are `--ctl` (selects too: they are
   drawn, not native, so they match), rows `--row`, settings rows at least
   `--ctl` plus `--sp-2` above and below. A hairline between rows is an inset
   shadow, so it adds no height. Hit targets are at least 24 px; a close button
   in a tab or row is a full `--ctl`.
5. **Nothing moves on hover.** Buttons that appear on hover in a list row are
   hidden, not removed, so the text beside them stays put. (A tab's close
   button is the one exception: it takes room from the title, whose fade moves;
   nothing else does.)
6. **Corners nest.** An inner radius is the outer radius minus the padding
   between them: `max(0px, var(--r-lg) - var(--sp-2))` for a menu item in a
   menu, `var(--r-md) - var(--sp-1)` for a close button in a tab.
7. **Trailing buttons sit like padding.** A row with a button at its end keeps
   `--sp-1` between the button and the row's edge, so the button is inset
   equally on three sides.
8. **Content never touches an edge, and overflow is handled.** Long text fades
   (`--fade`) or wraps; a menu item grows to fit a wrapped label; a set of
   things that can grow past its box (the workspace switcher) scrolls instead
   of spilling out.
9. **The window buttons are part of the row.** Content after them starts at
   `--lights`, keeping the same 9 px from the last button that the buttons keep
   from each other (on Windows, content before them stops `--gap` short).

## Motion

Three durations and three curves. Pick by what is moving, not by taste.

| Token | Value | Use for |
| --- | --- | --- |
| `--d-1` | 90 ms | Feedback (hover, press, focus) and anything **leaving**. |
| `--d-2` | 150 ms | A part **entering**: a tab, a menu. |
| `--d-3` | 220 ms | A whole surface: the bar, the page card resizing, a sheet, a workspace swap. |
| `--ease-out` | `cubic-bezier(.16, 1, .3, 1)` | Entering: most of the way at once, then settle. |
| `--ease-in` | `cubic-bezier(.4, 0, 1, 1)` | Leaving: gone without a wind-up. |
| `--ease-move` | `cubic-bezier(.2, .8, .2, 1)` | Moving between two places. |

Rules:

1. **Leaving is faster than entering.** Exits use `--d-1`; entrances `--d-2`
   or `--d-3`. Things get out of the way quickly and arrive gently.
2. **Move a short distance.** Things travel 6–18 px and fade; nothing crosses
   the window. Speed comes from short travel, not from rushing a long one.
3. **Direction means something.** The bar comes from the edge it lives on.
   Tabs slide in from the side the new workspace sits on. Menus grow from the
   point that opened them.
4. **Neighbors make room.** A new tab grows from zero height so the list below
   glides down; a closed tab folds away so the list glides up. Nothing jumps.
5. **Only new things animate.** Tabs are keyed by id, so a title update never
   replays an entrance.
6. **Pressing answers at once.** Buttons press to 96% within 40 ms and tabs to
   98.5% on mouse-down; the response starts on the press, not the release.
7. **Reduced motion is honored.** With the system setting on, all three
   durations become 1 ms, and the page card jumps to its new size instead of
   gliding (the interface only asks for a glide while `--d-3` is longer than
   20 ms, so a custom `--d-3: 0ms` does the same).

### Where motion is used

| Interaction | Animation |
| --- | --- |
| Collapsed bar revealed (cursor within the user's reveal distance of its window edge, 32 px by default, or past the edge; checked 40 times a second near the edge, less often the further away the cursor is) | The bar floats in over the page as a panel: slides 18 px from its edge, scales from 99%, fades in. `--d-3` `--ease-out` |
| Revealed bar let go (cursor away for 180 ms) | Slides back 11 px and fades. `--d-1` `--ease-in` |
| Bar collapsed or pinned open, layout changed | The page card **glides** to its new size over `--d-3` with the same soft landing (driven natively, `PAGE_GLIDE` in `main.rs`) while the bar slides in over `--d-3` |
| New tab / closed tab | Grows from / folds to zero height with a fade. `--d-2` in, `--d-1` out |
| Dragging a tab | The tab lifts (`--shadow-pop`) and follows the pointer inside its own group; the tabs it passes slide one place over, `--d-2` `--ease-move`. Let go and everything glides home |
| Pinning, unpinning, a new order from elsewhere | Every tab glides from where it was to where it now is, `--d-2` `--ease-move`. A pinned tile itself appears and goes by scaling from 80% |
| Switching workspace | Tab list slides 16 px from that workspace's side, `--d-3`; the window tint changes with it |
| Menu or workspace editor | Scales from 96% at its anchor, `--d-2` |
| Settings / Customize / History / Bookmarks sheet | Rises 6 px and fades in, `--d-3` |
| Settings section chosen | The new section rises 6 px and fades in, `--d-2`; the old one is gone at once |
| Suggestions under the address bar | Scales from 96% at the bar, `--d-2`; the chosen row is raised (`--surface`, `--shadow-1`) |
| Find bar opened (Mod+F) or closed | A row appears between the hotbar and the page: it drops 8 px and fades in over `--d-2` while the page card glides down to make room |
| Address bar focus | Lifts to `--surface`, accent ring grows, `--d-2` |
| Media player appears or goes (another tab starts or stops having media) | Rises 6 px and fades in, `--d-2` `--ease-out`; leaves the same way over `--d-1` `--ease-in`. Its progress line runs as one transition between reports, in steps of about a pixel (at most four a second), so nothing polls while it plays |
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
- The suggestions list under the address bar is glass, like a menu. Under a
  docked bar it is drawn by the main interface, so the page is hidden while it
  is open; under a floating (revealed) bar it floats over the page, and the
  floating view grows downwards to make room (`store::Msg::Grow`).
- Lists of pages (suggestions, History, Bookmarks) use one row, `.lrow`: a
  mark, the title, the address in `--mute`, a note, and buttons that appear on
  hover. Rows show a letter, not the site's icon, so opening a list contacts
  no site.
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

## Settings

Settings is one sheet, modelled on a preferences window: a menu of sections down
the side (a line icon and a name each, the current one raised like an active
tab) and the chosen section beside it, under a "Find in Settings" box. Only one
section shows at a time. When the page card is narrower than 720 px (a
container query on the sheet, so it follows the card, not the window) the menu
becomes a row of icon tabs above the content, and the section is headed by
its name.

Sections are grouped by what a person looks for, not by how the config is
stored: Appearance (layout, theme), Workspaces, Tabs and browsing, Search,
Privacy and security, Shortcuts, About and updates. The section in use is
remembered while Wisp runs; `openSettings(name)` opens Settings at one.

**Search** filters, across all sections, to the rows whose text or
`data-keys` contain every word typed, shown under their section names. A
section's or group's own name matching shows all of it. Escape empties the
box, then closes Settings. Typing a letter anywhere outside a field starts a
search, and Mod+F focuses the box.

**Simple and Advanced.** A switch beside the search box (remembered in the
config, `behavior.advanced_settings`; Simple by default). Simple is about
choices, not parameters: the things a person picks from, drawn as tiles.
Advanced shows everything, raw fields included. Both edit the same config. A
row, group or section with class `adv` shows only in Advanced; a search in
Simple still finds those and names them under the results, with a button to
show them.

**Tiles** are the one component for a pick-one choice: the layout, light or
dark, text size, search engine, what opens at start and in a new tab, and the
Simple/Advanced switch itself (`.seg`, the same component drawn as a segmented
control). Each is a radio group: a small picture (a layout drawing, a line
icon, or a plain lettermark for a search engine, never a copied logo) over its
name, the chosen one outlined in `--accent`. Arrow keys move, Space or Enter
chooses. A value set in Advanced that matches no tile shows as a checked
"Custom" tile, which opens the advanced field rather than overwriting it.
Choices live in `TILES` in `ui.html`.

**Keyboard**: the menu is one tab stop; the arrow keys (either direction, so
the same in the row of tabs), Home and End move through it.

**Adding a setting**: drop a row into the card of the group it belongs to, in
`src/ui.html` under the `SETTINGS` comment:

```html
<label class="row" data-keys="words people search for"><span>Label<small>What it does</small></span>
  <input type="checkbox" data-k="section.key"></label>
```

Add `adv` to its class if only Advanced should show it. For a pick-one setting,
add a tile row (`<div class="tiles" data-tiles="section.key" aria-label="…">`)
and its choices in `TILES`, and keep the raw field as an `adv` row.
`data-k` binds it to the config; the search reads the row's text and
`data-keys` (add the words the label alone would miss: "dark mode",
"ad blocker", "version"). A new group is a `.grp` (an `h2` and a card) inside a
section; a new section is a `.sec` with `data-sec`, `data-name` and
`data-icon`, and the menu is built from the sections.

## Direct manipulation before settings

If something can be adjusted by touching the thing itself, intuitively and with
no effort, it does not get a setting. The sidebar's width is set by dragging
its inner edge (double-click resets it); the bar is collapsed with its own
button, its hotkey or its right-click menu; tabs are reordered by dragging them;
toolbar items are placed by dragging in Customize; a workspace is edited from
its own icon. Settings are for choices that have no natural place to grab:
defaults, privacy, power.

Before adding a row to Settings, ask where the user would reach for it. If the
answer is a place on screen, build it there instead.

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
   and with a preset background. Screenshot runs never take focus, so
   focus rings (`:focus-within`) do not show in them.
