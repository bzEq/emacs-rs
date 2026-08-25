//! Emacs-style window tree: leaves display buffers, pairs split the area.

use crate::view::View;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// C-x 2: stacked, one above the other.
    Vertical,
    /// C-x 3: side by side.
    Horizontal,
}

/// A screen rect in terminal cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

#[derive(Debug, Clone)]
pub struct Window {
    /// Buffer id displayed in this window.
    pub buffer: usize,
    pub view: View,
    /// Buffer point while this window is not selected (window-point).
    pub point: Option<usize>,
}

impl Window {
    pub fn new(buffer: usize) -> Self {
        Window {
            buffer,
            view: View::new(),
            point: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Node {
    Leaf(Window),
    Pair(Split, Box<Node>, Box<Node>),
}

#[derive(Debug, Clone)]
pub struct WindowTree {
    root: Node,
    /// Path from root to the selected leaf: child index at each Pair level.
    path: Vec<usize>,
}

impl WindowTree {
    pub fn new(buffer: usize) -> Self {
        WindowTree {
            root: Node::Leaf(Window::new(buffer)),
            path: Vec::new(),
        }
    }

    fn node_at(&self, path: &[usize]) -> &Node {
        let mut node = &self.root;
        for &i in path {
            if let Node::Pair(_, a, b) = node {
                node = if i == 0 { a } else { b };
            } else {
                unreachable!("window path ends before tree depth");
            }
        }
        node
    }

    fn node_at_mut(&mut self, path: &[usize]) -> &mut Node {
        let mut node = &mut self.root;
        for &i in path {
            if let Node::Pair(_, a, b) = node {
                node = if i == 0 { a } else { b };
            } else {
                unreachable!("window path ends before tree depth");
            }
        }
        node
    }

    pub fn selected(&self) -> &Window {
        match self.node_at(&self.path) {
            Node::Leaf(w) => w,
            Node::Pair(..) => unreachable!("path must lead to a leaf"),
        }
    }

    pub fn selected_mut(&mut self) -> &mut Window {
        let path = self.path.clone();
        match self.node_at_mut(&path) {
            Node::Leaf(w) => w,
            Node::Pair(..) => unreachable!("path must lead to a leaf"),
        }
    }

    pub fn selected_buffer(&self) -> usize {
        self.selected().buffer
    }

    pub fn selected_path(&self) -> &[usize] {
        &self.path
    }

    pub fn is_single(&self) -> bool {
        matches!(self.root, Node::Leaf(_))
    }

    /// Split the selected window; the new window shows the same buffer and
    /// becomes selected. Both resulting windows remember `point` as their
    /// own window-point.
    pub fn split(&mut self, split: Split, point: usize) {
        let mut cur = self.selected().clone();
        cur.point = Some(point);
        let old_leaf = Node::Leaf(cur.clone());
        let new_leaf = Node::Leaf(cur);
        let pair = match split {
            Split::Vertical => Node::Pair(split, Box::new(old_leaf), Box::new(new_leaf)),
            Split::Horizontal => Node::Pair(split, Box::new(old_leaf), Box::new(new_leaf)),
        };
        let path_to_parent = {
            let mut p = self.path.clone();
            p.pop();
            p
        };
        let idx = self.path.last().copied();
        match (idx, self.path.is_empty()) {
            (Some(i), false) => {
                let node = self.node_at_mut(&path_to_parent);
                if let Node::Pair(_, a, b) = node {
                    let slot = if i == 0 { a } else { b };
                    **slot = pair;
                }
            }
            _ => {
                self.root = pair;
            }
        }
        self.path.push(1);
    }

    /// Delete the selected window; its sibling takes its place. Returns false
    /// if it is the sole window.
    pub fn delete_selected(&mut self) -> bool {
        if self.path.is_empty() {
            return false;
        }
        let idx = self.path.pop().unwrap();
        // `path` now points at the parent pair; remove the sibling child.
        let path = self.path.clone();
        let node = self.node_at_mut(&path);
        if let Node::Pair(_, a, b) = node {
            let sibling = if idx == 0 { b } else { a };
            let sibling = sibling.clone();
            *node = *sibling;
        }
        // The collapse point may now hold a pair: descend to its first leaf
        // so the selected path still reaches a window.
        loop {
            let is_pair = matches!(self.node_at(&self.path), Node::Pair(..));
            if !is_pair {
                break;
            }
            self.path.push(0);
        }
        true
    }

    /// Delete all windows but the selected one (C-x 1).
    pub fn delete_others(&mut self) {
        let cur = self.selected().clone();
        self.root = Node::Leaf(cur);
        self.path.clear();
    }

    /// Cycle to the next window in tree order (C-x o). Returns false if
    /// there is only one window.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> bool {
        let leaves = self.leaf_paths();
        if leaves.len() <= 1 {
            return false;
        }
        let pos = leaves.iter().position(|p| p == &self.path).unwrap();
        let next = (pos + 1) % leaves.len();
        self.path = leaves[next].clone();
        true
    }

    fn leaf_paths(&self) -> Vec<Vec<usize>> {
        let mut out = Vec::new();
        fn walk(node: &Node, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
            match node {
                Node::Leaf(_) => out.push(path.clone()),
                Node::Pair(_, a, b) => {
                    path.push(0);
                    walk(a, path, out);
                    *path.last_mut().unwrap() = 1;
                    walk(b, path, out);
                    path.pop();
                }
            }
        }
        walk(&self.root, &mut Vec::new(), &mut out);
        out
    }

    /// All leaf windows with their paths, in tree order.
    pub fn leaves(&self) -> Vec<&Window> {
        let mut out = Vec::new();
        fn walk<'a>(node: &'a Node, out: &mut Vec<&'a Window>) {
            match node {
                Node::Leaf(w) => out.push(w),
                Node::Pair(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
            }
        }
        walk(&self.root, &mut out);
        out
    }

    pub fn leaves_mut(&mut self) -> Vec<&mut Window> {
        let mut out = Vec::new();
        fn walk<'a>(node: &'a mut Node, out: &mut Vec<&'a mut Window>) {
            match node {
                Node::Leaf(w) => out.push(w),
                Node::Pair(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
            }
        }
        walk(&mut self.root, &mut out);
        out
    }

    /// Replace the buffer shown in all windows that display `old_buf` with
    /// `new_buf` (used when a buffer is killed).
    pub fn replace_buffer(&mut self, old_buf: usize, new_buf: usize) {
        for w in self.leaves_mut() {
            if w.buffer == old_buf {
                w.buffer = new_buf;
                w.point = None;
            }
        }
    }

    /// Lay out all leaves over `area`. Returns (path, window, rect) triples
    /// in tree order.
    pub fn layout(&self, area: Rect) -> Vec<(Vec<usize>, &Window, Rect)> {
        let mut out = Vec::new();
        fn walk<'a>(
            node: &'a Node,
            path: &mut Vec<usize>,
            area: Rect,
            out: &mut Vec<(Vec<usize>, &'a Window, Rect)>,
        ) {
            match node {
                Node::Leaf(w) => out.push((path.clone(), w, area)),
                Node::Pair(split, a, b) => {
                    let (ra, rb) = match split {
                        Split::Vertical => {
                            let h1 = area.h / 2;
                            let h2 = area.h - h1;
                            (
                                Rect { h: h1, ..area },
                                Rect {
                                    y: area.y + h1,
                                    h: h2,
                                    ..area
                                },
                            )
                        }
                        Split::Horizontal => {
                            let w1 = area.w / 2;
                            let w2 = area.w - w1;
                            (
                                Rect { w: w1, ..area },
                                Rect {
                                    x: area.x + w1,
                                    w: w2,
                                    ..area
                                },
                            )
                        }
                    };
                    path.push(0);
                    walk(a, path, ra, out);
                    *path.last_mut().unwrap() = 1;
                    walk(b, path, rb, out);
                    path.pop();
                }
            }
        }
        walk(&self.root, &mut Vec::new(), area, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_layout() {
        let mut t = WindowTree::new(1);
        t.split(Split::Vertical, 0);
        t.split(Split::Horizontal, 0);
        let area = Rect {
            x: 0,
            y: 0,
            w: 100,
            h: 50,
        };
        let l = t.layout(area);
        assert_eq!(l.len(), 3);
        // bottom window is horizontal-split
        assert_eq!(
            l[0].2,
            Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 25
            }
        );
        assert_eq!(
            l[1].2,
            Rect {
                x: 0,
                y: 25,
                w: 50,
                h: 25
            }
        );
        assert_eq!(
            l[2].2,
            Rect {
                x: 50,
                y: 25,
                w: 50,
                h: 25
            }
        );
    }

    #[test]
    fn delete_selected() {
        let mut t = WindowTree::new(1);
        t.split(Split::Vertical, 0);
        assert!(!t.is_single());
        assert!(t.delete_selected());
        assert!(t.is_single());
        assert!(!t.delete_selected(), "cannot delete sole window");
    }

    #[test]
    fn delete_root_child_collapses_to_sibling_pair() {
        let mut t = WindowTree::new(1);
        t.split(Split::Vertical, 0); // path [1]
        t.split(Split::Horizontal, 0); // path [1, 1]
        assert!(t.next()); // path [0]
        assert!(t.delete_selected());
        // sibling of the deleted root child is a horizontal pair; the
        // selected path must descend to a leaf
        let _ = t.selected();
        assert_eq!(
            t.layout(Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 50
            })
            .len(),
            2
        );
    }

    #[test]
    fn delete_others_and_next() {
        let mut t = WindowTree::new(1);
        t.split(Split::Vertical, 0);
        t.next();
        t.delete_others();
        assert!(t.is_single());
        assert!(!t.next(), "no other window to cycle to");
    }

    #[test]
    fn next_cycles() {
        let mut t = WindowTree::new(1);
        t.split(Split::Vertical, 0);
        t.split(Split::Horizontal, 0);
        let first = t.path.clone();
        assert!(t.next());
        assert!(t.next());
        assert!(t.next());
        assert_eq!(t.path, first, "cycles back to the start");
    }
}
