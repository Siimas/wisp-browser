//! Split view: several tabs of a workspace side by side in the page area.
//!
//! A split is a small tree: tiles (tabs) and splits that lay their children out in a
//! row or a column, each child taking a share (`sizes`) of the room. The window keeps
//! one tree per group of split tabs; the group of the focused tab is on screen. The
//! interface draws the same tree (cards, dividers, the drop preview) and the window
//! places each tab's view on its tile, so both use the same arithmetic: share of
//! (length - gaps), children separated by one gap.

use crate::{App, Tab, native, place_tab};
use serde::{Deserialize, Serialize};

/// Most tiles one split holds: past four, pages get too small to use.
pub const MAX_TILES: usize = 4;
/// A tile's id while the user is still dragging a tab over the page: the place the
/// tab would land, drawn by the interface.
pub const PREVIEW: u32 = 0;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(untagged)]
pub enum Node {
    Tab(u32),
    Split {
        /// Children side by side (true) or stacked (false).
        row: bool,
        sizes: Vec<f64>,
        kids: Vec<Node>,
    },
}

/// Which side of a tile a new tile goes on.
#[derive(Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    fn row(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
    fn before(self) -> bool {
        matches!(self, Edge::Left | Edge::Top)
    }
}

impl Node {
    /// The tabs, in reading order.
    pub fn tabs(&self) -> Vec<u32> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<u32>) {
        match self {
            Node::Tab(id) => out.push(*id),
            Node::Split { kids, .. } => kids.iter().for_each(|k| k.collect(out)),
        }
    }

    pub fn contains(&self, id: u32) -> bool {
        self.tabs().contains(&id)
    }

    /// Put tab `id` beside tab `target`, on its `edge`. Beside a tile that already sits
    /// in a row (or column) going the same way, it joins that row and the two share
    /// the target's room; otherwise the target's tile divides in two.
    pub fn with(self, target: u32, id: u32, edge: Edge) -> Node {
        let pair = |t: Node| {
            let (new, t) = (Node::Tab(id), t);
            let kids = if edge.before() {
                vec![new, t]
            } else {
                vec![t, new]
            };
            Node::Split {
                row: edge.row(),
                sizes: vec![1.0, 1.0],
                kids,
            }
        };
        match self {
            Node::Tab(t) if t == target => pair(Node::Tab(t)),
            leaf @ Node::Tab(_) => leaf,
            Node::Split {
                row,
                mut sizes,
                mut kids,
            } => {
                let here = kids.iter().position(|k| *k == Node::Tab(target));
                match here {
                    Some(i) if row == edge.row() => {
                        let half = sizes[i] / 2.0;
                        sizes[i] = half;
                        let at = if edge.before() { i } else { i + 1 };
                        sizes.insert(at, half);
                        kids.insert(at, Node::Tab(id));
                    }
                    _ => kids = kids.into_iter().map(|k| k.with(target, id, edge)).collect(),
                }
                Node::Split { row, sizes, kids }
            }
        }
    }

    /// Take tab `id` out. A split left with one child becomes that child, and a split
    /// left inside one going the same way melts into it; `None` once nothing is left.
    pub fn without(self, id: u32) -> Option<Node> {
        match self {
            Node::Tab(t) => (t != id).then_some(self),
            Node::Split { row, sizes, kids } => {
                let (mut s, mut k) = (Vec::new(), Vec::new());
                for (size, kid) in sizes.into_iter().zip(kids) {
                    match kid.without(id) {
                        // Same direction as this split: its children join this one.
                        Some(Node::Split {
                            row: r,
                            sizes: inner,
                            kids: ks,
                        }) if r == row => {
                            let sum: f64 = inner.iter().sum();
                            s.extend(inner.iter().map(|x| size * x / sum));
                            k.extend(ks);
                        }
                        Some(kid) => {
                            s.push(size);
                            k.push(kid);
                        }
                        None => {}
                    }
                }
                match k.len() {
                    0 => None,
                    1 => k.pop(),
                    _ => Some(Node::Split {
                        row,
                        sizes: s,
                        kids: k,
                    }),
                }
            }
        }
    }

    /// Keep only the tabs `keep` says yes to.
    pub fn retain(self, keep: impl Fn(u32) -> bool) -> Option<Node> {
        let gone: Vec<u32> = self.tabs().into_iter().filter(|&t| !keep(t)).collect();
        gone.into_iter().try_fold(self, |n, t| n.without(t))
    }

    /// Rename every tab; `None` from `f` drops the whole tree.
    pub fn relabel(self, f: &impl Fn(u32) -> Option<u32>) -> Option<Node> {
        Some(match self {
            Node::Tab(t) => Node::Tab(f(t)?),
            Node::Split { row, sizes, kids } => Node::Split {
                row,
                sizes,
                kids: kids
                    .into_iter()
                    .map(|k| k.relabel(f))
                    .collect::<Option<_>>()?,
            },
        })
    }

    /// The tab nearest to `id` in reading order (the one before it, else after): who
    /// takes over when `id` leaves.
    pub fn neighbor(&self, id: u32) -> Option<u32> {
        let tabs = self.tabs();
        let i = tabs.iter().position(|&t| t == id)?;
        i.checked_sub(1)
            .and_then(|j| tabs.get(j))
            .or(tabs.get(i + 1))
            .copied()
    }

    /// The split at `path` (child indices from the top).
    pub fn at(&mut self, path: &[usize]) -> Option<&mut Node> {
        match path.split_first() {
            None => Some(self),
            Some((&i, rest)) => match self {
                Node::Split { kids, .. } => kids.get_mut(i)?.at(rest),
                Node::Tab(_) => None,
            },
        }
    }

    /// Something to show: two to `MAX_TILES` different tabs, every split with two or
    /// more children and a usable share for each.
    pub fn valid(&self) -> bool {
        let mut tabs = self.tabs();
        let n = tabs.len();
        tabs.sort_unstable();
        tabs.dedup();
        tabs.len() == n && (2..=MAX_TILES).contains(&n) && self.sane()
    }

    fn sane(&self) -> bool {
        match self {
            Node::Tab(_) => true,
            Node::Split { sizes, kids, .. } => {
                kids.len() >= 2
                    && sizes.len() == kids.len()
                    && sizes.iter().all(|s| s.is_finite() && *s > 0.0)
                    && kids.iter().all(Node::sane)
            }
        }
    }

    /// Where each tab goes inside `[x, y, w, h]`, with `gap` between tiles.
    pub fn frames(&self, [x, y, w, h]: [f64; 4], gap: f64, out: &mut Vec<(u32, [f64; 4])>) {
        match self {
            Node::Tab(id) => out.push((*id, [x, y, w, h])),
            Node::Split { row, sizes, kids } => {
                let total: f64 = sizes.iter().sum();
                let room = (if *row { w } else { h }) - gap * (kids.len() - 1) as f64;
                let mut at = if *row { x } else { y };
                for (kid, size) in kids.iter().zip(sizes) {
                    let len = room * size / total;
                    let frame = if *row {
                        [at, y, len, h]
                    } else {
                        [x, at, w, len]
                    };
                    kid.frames(frame, gap, out);
                    at += len + gap;
                }
            }
        }
    }
}

/// The groups as saved: tabs named by their place in the saved tab list. A tab that is
/// not saved (private, blank) leaves its group.
pub fn save(groups: &[Node], index_of: impl Fn(u32) -> Option<u32>) -> Vec<Node> {
    groups
        .iter()
        .filter_map(|g| g.clone().retain(|t| index_of(t).is_some()))
        .filter_map(|g| g.relabel(&index_of))
        .filter(Node::valid)
        .collect()
}

/// The groups of a saved session, with `id_of` turning places in the saved tab list
/// into tab ids. Anything that does not make sense is dropped.
pub fn load(saved: Vec<Node>, id_of: impl Fn(u32) -> Option<u32>) -> Vec<Node> {
    let mut used = Vec::new();
    saved
        .into_iter()
        .filter_map(|g| g.relabel(&id_of))
        .filter(|g| {
            let fresh = g.valid() && g.tabs().iter().all(|t| !used.contains(t));
            if fresh {
                used.extend(g.tabs());
            }
            fresh
        })
        .collect()
}

// ---------------------------------------------------------------- the window's side

/// Can this tab be a tile at all? Pinned tabs keep to their own box in the tab list,
/// so they stay out of splits.
fn can_tile(tab: &Tab) -> bool {
    !tab.pinned
}

impl App {
    /// Index in `groups` of the split holding tab `id`.
    pub(crate) fn group_of(&self, id: u32) -> Option<usize> {
        self.groups.iter().position(|g| g.contains(id))
    }

    /// The tree on screen: the drop preview while a tab is dragged over the page,
    /// otherwise the split of the focused tab, if it is in one.
    pub(crate) fn shown_tree(&self) -> Option<&Node> {
        self.preview
            .as_ref()
            .or_else(|| self.groups.get(self.group_of(self.active?)?))
    }

    /// Groups of the workspace on screen, as lists of tabs in reading order.
    pub(crate) fn groups_json(&self) -> Vec<Vec<u32>> {
        let here = |g: &&Node| {
            g.tabs()
                .first()
                .and_then(|&id| self.tab(id))
                .is_some_and(|t| t.workspace == self.workspace)
        };
        self.groups.iter().filter(here).map(Node::tabs).collect()
    }

    /// Where the visible tabs go once any glide is over: tab id and `[x, y, w, h]`.
    pub(crate) fn targets(&self) -> Vec<(u32, [f64; 4])> {
        let s = self.size();
        let [l, t, r, b] = self.insets.unwrap_or_default();
        let page = [l, t, s.width - l - r, s.height - t - b];
        let mut out = Vec::new();
        match self.shown_tree() {
            Some(tree) => tree.frames(page, self.gap, &mut out),
            None => out.extend(self.active.map(|id| (id, page))),
        }
        out.retain(|&(id, _)| id != PREVIEW);
        out
    }

    /// Give tab `id` its view if it has none yet (restored tabs load when first shown).
    pub(crate) fn ensure_webview(&mut self, id: u32) {
        let Some(tab) = self.tab(id).filter(|t| t.webview.is_none()) else {
            return;
        };
        let (url, private) = (tab.url.clone(), tab.private);
        let webview = self.build_webview(id, &url, private);
        if let Some(tab) = self.tab_mut(id) {
            tab.webview = webview;
        }
    }

    /// Put tab `id` right after tab `after` in the list, so a split reads as one unit.
    fn place_after(&mut self, id: u32, after: u32) {
        let ws = self.tab(after).map(|t| t.workspace);
        let peers = self
            .tabs
            .iter()
            .filter(|t| Some(t.workspace) == ws && t.id != id);
        if let Some(i) = peers.map(|t| t.id).position(|t| t == after) {
            place_tab(&mut self.tabs, id, i + 1);
        }
    }

    /// Take tab `id` out of its split (the others stay split if two or more remain).
    /// Gives the tab that takes over its place.
    pub(crate) fn leave_split(&mut self, id: u32) -> Option<u32> {
        let g = self.group_of(id)?;
        if self.shown_tree() == self.groups.get(g) {
            self.start_glide();
        }
        let tree = self.groups.remove(g);
        let near = tree.neighbor(id);
        if let Some(rest @ Node::Split { .. }) = tree.without(id) {
            self.groups.insert(g, rest);
        }
        self.layout_pages();
        near
    }

    /// Show tab `id` beside `target` (on `edge`, or along the target tile's longer
    /// side), joining or starting the split `target` is in, and focus it.
    pub(crate) fn split_with(&mut self, id: u32, target: u32, edge: Option<Edge>) {
        let (Some(a), Some(b)) = (self.tab(id), self.tab(target)) else {
            return;
        };
        if id == target || a.workspace != b.workspace || !can_tile(a) || !can_tile(b) {
            return;
        }
        let edge = edge.unwrap_or_else(|| {
            let frame = self.targets().into_iter().find(|f| f.0 == target);
            match frame {
                Some((_, [.., w, h])) if h > w => Edge::Bottom,
                _ => Edge::Right,
            }
        });
        let same = self.group_of(target).is_some() && self.group_of(target) == self.group_of(id);
        let full = self
            .group_of(target)
            .is_some_and(|g| self.groups[g].tabs().len() >= MAX_TILES);
        if full && !same {
            return;
        }
        self.start_glide();
        self.preview = None;
        if !same {
            self.leave_split(id);
        }
        let tree = match self.group_of(target) {
            Some(g) => self.groups.remove(g),
            None => Node::Tab(target),
        };
        let tree = tree.without(id).unwrap_or(Node::Tab(target));
        let tree = tree.with(target, id, edge);
        let tabs = tree.tabs();
        self.groups.push(tree);
        // Keep the group together in the list, in reading order, where its first tab is.
        let first = self
            .tabs
            .iter()
            .find(|t| tabs.contains(&t.id))
            .map(|t| t.id);
        let mut prev = first.unwrap_or(target);
        for &t in tabs.iter().filter(|&&t| Some(t) != first) {
            self.place_after(t, prev);
            prev = t;
        }
        self.select(id);
        self.save_session();
    }

    /// Take tab `id` out of its split, or (`all`) end the split altogether. Tabs stay open.
    pub(crate) fn unsplit(&mut self, id: u32, all: bool) {
        let Some(g) = self.group_of(id) else { return };
        if all {
            if self.shown_tree() == self.groups.get(g) {
                self.start_glide();
            }
            self.groups.remove(g);
            self.layout_pages();
            self.sync();
        } else {
            let near = self.leave_split(id);
            match near.filter(|_| self.active == Some(id)) {
                Some(next) => self.select(next),
                None => self.sync(),
            }
        }
        self.save_session();
    }

    /// The hotkey: split the focused tab with the one after it in the list (or before
    /// it, at the end), or end the split it is in.
    pub(crate) fn toggle_split(&mut self) {
        let Some(id) = self.active else { return };
        if self.group_of(id).is_some() {
            return self.unsplit(id, true);
        }
        let ids = self.workspace_tabs();
        let i = ids.iter().position(|&t| t == id).unwrap_or(0);
        let free = |t: &&u32| {
            **t != id && self.group_of(**t).is_none() && self.tab(**t).is_some_and(can_tile)
        };
        let partner = ids[i..]
            .iter()
            .find(free)
            .or(ids[..i].iter().rev().find(free));
        if let Some(&partner) = partner {
            self.split_with(partner, id, None);
            self.select(id); // the focus stays where it was
        }
    }

    /// A tab is being dragged over the page: show where it would land (`target` and
    /// `edge`), or nothing.
    pub(crate) fn split_preview(&mut self, id: u32, target: Option<(u32, Edge)>) {
        let tree = target.and_then(|(t, edge)| self.preview_tree(id, t, edge));
        if tree == self.preview {
            return;
        }
        self.start_glide();
        self.preview = tree;
        self.layout_pages();
        self.sync();
    }

    fn preview_tree(&self, id: u32, target: u32, edge: Edge) -> Option<Node> {
        let (a, b) = (self.tab(id)?, self.tab(target)?);
        if id == target || a.workspace != b.workspace || !can_tile(a) || !can_tile(b) {
            return None;
        }
        let active = self.active?;
        let shown = self.group_of(active).map(|g| self.groups[g].clone());
        let base = shown.unwrap_or(Node::Tab(active));
        if !base.contains(target) {
            return None;
        }
        let base = if base.contains(id) {
            base.without(id)?
        } else if base.tabs().len() >= MAX_TILES {
            return None;
        } else {
            base
        };
        Some(base.with(target, PREVIEW, edge))
    }

    /// A divider was dragged (or double-clicked): new shares for the split at `path`
    /// of the split on screen.
    pub(crate) fn split_sizes(&mut self, path: &[usize], sizes: Vec<f64>, animate: bool) {
        if self.preview.is_some() {
            return;
        }
        let Some(g) = self.active.and_then(|id| self.group_of(id)) else {
            return;
        };
        let mut tree = self.groups[g].clone();
        if let Some(Node::Split { sizes: s, .. }) = tree.at(path)
            && s.len() == sizes.len()
        {
            *s = sizes;
        }
        if !tree.valid() || tree == self.groups[g] {
            return;
        }
        if animate {
            self.start_glide();
        }
        self.groups[g] = tree;
        self.layout_pages();
    }

    /// A page was clicked. If it is a tile on screen that really holds the keyboard
    /// focus now (asked of the system, not the page), it becomes the focused tile.
    pub(crate) fn page_focus(&mut self, id: u32) {
        if self.active == Some(id) || !self.page_visible() {
            return;
        }
        let on_screen = self.preview.is_none() && self.shown_tree().is_some_and(|t| t.contains(id));
        let focused = self
            .tab(id)
            .and_then(|t| t.webview.as_ref())
            .is_some_and(|w| native::is_focused(w, &self.window));
        if on_screen && focused {
            self.active = Some(id);
            self.last_active.insert(self.workspace, id);
            self.sync();
        }
    }

    /// Splits whose tabs went away, were pinned, or are spread over workspaces (a
    /// hand-edited session) lose those tabs.
    pub(crate) fn tidy_groups(&mut self) {
        let groups = std::mem::take(&mut self.groups);
        for g in groups {
            let ws = g
                .tabs()
                .first()
                .and_then(|&t| self.tab(t))
                .map(|t| t.workspace);
            let ok = |t: u32| {
                self.tab(t)
                    .is_some_and(|t| can_tile(t) && Some(t.workspace) == ws)
            };
            if let Some(g) = g.retain(ok).filter(Node::valid) {
                self.groups.push(g);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: u32) -> Node {
        Node::Tab(id)
    }
    fn split(row: bool, kids: Vec<Node>) -> Node {
        Node::Split {
            row,
            sizes: vec![1.0; kids.len()],
            kids,
        }
    }

    #[test]
    fn tiles_join_rows_and_columns() {
        let t = tab(1).with(1, 2, Edge::Right);
        assert_eq!(t, split(true, vec![tab(1), tab(2)]));
        // Beside a tile in a row, going the same way: it joins the row, halving that tile.
        let t = t.with(2, 3, Edge::Left);
        assert_eq!(t.tabs(), [1, 3, 2]);
        let Node::Split { ref sizes, .. } = t else {
            panic!()
        };
        assert_eq!(sizes, &[1.0, 0.5, 0.5]);
        // Across it: that tile divides into a column.
        let t = t.with(2, 4, Edge::Bottom);
        assert_eq!(
            t,
            Node::Split {
                row: true,
                sizes: vec![1.0, 0.5, 0.5],
                kids: vec![tab(1), tab(3), split(false, vec![tab(2), tab(4)])],
            }
        );
        assert!(t.valid());
        // An unknown target changes nothing.
        assert_eq!(tab(1).with(9, 2, Edge::Top), tab(1));
    }

    #[test]
    fn leaving_collapses_and_melts() {
        let t = tab(1)
            .with(1, 2, Edge::Right)
            .with(2, 3, Edge::Bottom)
            .with(1, 4, Edge::Top);
        // [col[4, 1], col[2, 3]] in a row.
        assert_eq!(t.tabs(), [4, 1, 2, 3]);
        let t = t.without(3).unwrap();
        assert_eq!(t.tabs(), [4, 1, 2]);
        // The column [2] became a plain tile.
        let Node::Split { ref kids, .. } = t else {
            panic!()
        };
        assert_eq!(kids[1], tab(2));
        // Out goes 1: the column [4] collapses, and the row is [4, 2].
        let t = t.without(1).unwrap();
        assert_eq!(t, split(true, vec![tab(4), tab(2)]));
        assert_eq!(t.clone().without(4), Some(tab(2)));
        assert_eq!(tab(2).without(2), None);
        // A row inside a row (left after its column went) melts into it, keeping shares.
        let t = Node::Split {
            row: true,
            sizes: vec![2.0, 1.0],
            kids: vec![
                Node::Split {
                    row: false,
                    sizes: vec![1.0, 1.0],
                    kids: vec![tab(5), split(true, vec![tab(1), tab(2)])],
                },
                tab(3),
            ],
        };
        assert_eq!(
            t.without(5).unwrap(),
            Node::Split {
                row: true,
                sizes: vec![1.0, 1.0, 1.0],
                kids: vec![tab(1), tab(2), tab(3)],
            }
        );
    }

    #[test]
    fn closing_a_tile_hands_over_to_its_neighbor() {
        let t = tab(1).with(1, 2, Edge::Right).with(2, 3, Edge::Right);
        assert_eq!(t.neighbor(1), Some(2)); // first: the one after
        assert_eq!(t.neighbor(3), Some(2)); // otherwise the one before
        assert_eq!(t.neighbor(2), Some(1));
        assert_eq!(t.neighbor(9), None);
    }

    #[test]
    fn frames_fill_the_page_with_gaps() {
        let t = Node::Split {
            row: true,
            sizes: vec![3.0, 1.0],
            kids: vec![tab(1), split(false, vec![tab(2), tab(3)])],
        };
        let mut out = Vec::new();
        t.frames([10.0, 20.0, 808.0, 408.0], 8.0, &mut out);
        assert_eq!(
            out,
            [
                (1, [10.0, 20.0, 600.0, 408.0]),
                (2, [618.0, 20.0, 200.0, 200.0]),
                (3, [618.0, 228.0, 200.0, 200.0]),
            ]
        );
    }

    #[test]
    fn only_sensible_trees_are_valid() {
        assert!(!tab(1).valid()); // one tile is no split
        assert!(!split(true, vec![tab(1), tab(1)]).valid());
        assert!(!split(true, (1..=5).map(tab).collect()).valid());
        let bad = Node::Split {
            row: true,
            sizes: vec![1.0, f64::NAN],
            kids: vec![tab(1), tab(2)],
        };
        assert!(!bad.valid());
        let mut t = split(true, vec![tab(1), split(false, vec![tab(2), tab(3)])]);
        assert!(t.at(&[1]).is_some() && t.at(&[0, 0]).is_none() && t.at(&[2]).is_none());
    }

    #[test]
    fn sessions_round_trip() {
        // Tabs 10, 11, 12, 13 saved at places 0..4; tab 14 is private (not saved).
        let groups = vec![
            tab(10).with(10, 11, Edge::Right).with(11, 14, Edge::Bottom),
            tab(12).with(12, 14, Edge::Left), // only one saved tab left: no split
        ];
        let index_of = |id: u32| (10..14).contains(&id).then(|| id - 10);
        let saved = save(&groups, index_of);
        assert_eq!(saved, [split(true, vec![tab(0), tab(1)])]);
        let json = serde_json::to_string(&saved).unwrap();
        assert_eq!(json, r#"[{"row":true,"sizes":[1.0,1.0],"kids":[0,1]}]"#);

        // Back in: places become the new ids; nonsense is dropped.
        let back: Vec<Node> = serde_json::from_str(&json).unwrap();
        let ids = |i: u32| (i < 4).then_some(100 + i);
        assert_eq!(load(back, ids), [split(true, vec![tab(100), tab(101)])]);
        let junk = vec![
            split(true, vec![tab(0), tab(7)]), // a place that is not there
            split(true, vec![tab(2), tab(2)]), // twice the same tab
            split(true, vec![tab(1), tab(3)]),
            split(false, vec![tab(3), tab(0)]), // tab 3 is already in a split
        ];
        assert_eq!(load(junk, ids), [split(true, vec![tab(101), tab(103)])]);
    }
}
