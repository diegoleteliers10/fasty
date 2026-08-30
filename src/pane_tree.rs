use std::sync::Arc;
use gpui::{Bounds, Pixels};
use crate::git::GitStatus;
use crate::terminal_state::TerminalState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    Horizontal, // Side-by-side (Left | Right)
    Vertical,   // Stacked (Top / Down)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Top,
    Down,
}

pub type PaneId = usize;

pub struct TerminalPane {
    pub id: PaneId,
    pub terminal: Option<Arc<TerminalState>>,
    pub title: String,
    pub custom_title: Option<String>,
    pub cwd: Option<std::path::PathBuf>,
    pub git_status: Option<GitStatus>,
    pub git_checked_cwd: Option<std::path::PathBuf>,
    pub git_last_poll: Option<std::time::Instant>,
    pub last_duration_ms: Option<u128>,
    pub last_exit_code: Option<i32>,
    pub last_bounds: Option<Bounds<Pixels>>,
}

impl Clone for TerminalPane {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            terminal: self.terminal.clone(),
            title: self.title.clone(),
            custom_title: self.custom_title.clone(),
            cwd: self.cwd.clone(),
            git_status: self.git_status.clone(),
            git_checked_cwd: self.git_checked_cwd.clone(),
            git_last_poll: self.git_last_poll,
            last_duration_ms: self.last_duration_ms,
            last_exit_code: self.last_exit_code,
            last_bounds: self.last_bounds,
        }
    }
}

pub enum PaneNode {
    Leaf(TerminalPane),
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

impl PaneNode {
    pub fn split_at_target(
        self,
        target_id: PaneId,
        new_pane: TerminalPane,
        direction: Direction,
    ) -> Self {
        match self {
            PaneNode::Leaf(pane) => {
                if pane.id == target_id {
                    let split_dir = match direction {
                        Direction::Left | Direction::Right => SplitDirection::Horizontal,
                        Direction::Top | Direction::Down => SplitDirection::Vertical,
                    };
                    let (first, second) = match direction {
                        Direction::Right | Direction::Down => {
                            (Box::new(PaneNode::Leaf(pane)), Box::new(PaneNode::Leaf(new_pane)))
                        }
                        Direction::Left | Direction::Top => {
                            (Box::new(PaneNode::Leaf(new_pane)), Box::new(PaneNode::Leaf(pane)))
                        }
                    };
                    PaneNode::Split {
                        direction: split_dir,
                        ratio: 0.5,
                        first,
                        second,
                    }
                } else {
                    PaneNode::Leaf(pane)
                }
            }
            PaneNode::Split {
                direction: d,
                ratio,
                first,
                second,
            } => PaneNode::Split {
                direction: d,
                ratio,
                first: Box::new(first.split_at_target(target_id, new_pane.clone(), direction)),
                second: Box::new(second.split_at_target(target_id, new_pane, direction)),
            },
        }
    }

    pub fn close_at_target(self, target_id: PaneId) -> Option<Self> {
        match self {
            PaneNode::Leaf(pane) => {
                if pane.id == target_id {
                    None
                } else {
                    Some(PaneNode::Leaf(pane))
                }
            }
            PaneNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let first_opt = first.close_at_target(target_id);
                let second_opt = second.close_at_target(target_id);
                match (first_opt, second_opt) {
                    (Some(f), Some(s)) => Some(PaneNode::Split {
                        direction,
                        ratio,
                        first: Box::new(f),
                        second: Box::new(s),
                    }),
                    (Some(f), None) => Some(f),
                    (None, Some(s)) => Some(s),
                    (None, None) => None,
                }
            }
        }
    }

    pub fn find_pane(&self, id: PaneId) -> Option<&TerminalPane> {
        match self {
            PaneNode::Leaf(pane) => {
                if pane.id == id {
                    Some(pane)
                } else {
                    None
                }
            }
            PaneNode::Split { first, second, .. } => {
                first.find_pane(id).or_else(|| second.find_pane(id))
            }
        }
    }

    pub fn find_pane_mut(&mut self, id: PaneId) -> Option<&mut TerminalPane> {
        match self {
            PaneNode::Leaf(pane) => {
                if pane.id == id {
                    Some(pane)
                } else {
                    None
                }
            }
            PaneNode::Split { first, second, .. } => {
                if let Some(p) = first.find_pane_mut(id) {
                    Some(p)
                } else {
                    second.find_pane_mut(id)
                }
            }
        }
    }

    pub fn collect_panes<'a>(&'a self, list: &mut Vec<&'a TerminalPane>) {
        match self {
            PaneNode::Leaf(pane) => list.push(pane),
            PaneNode::Split { first, second, .. } => {
                first.collect_panes(list);
                second.collect_panes(list);
            }
        }
    }

    pub fn all_panes(&self) -> Vec<&TerminalPane> {
        let mut list = Vec::new();
        self.collect_panes(&mut list);
        list
    }

    pub fn collect_panes_mut<'a>(&'a mut self, list: &mut Vec<&'a mut TerminalPane>) {
        match self {
            PaneNode::Leaf(pane) => list.push(pane),
            PaneNode::Split { first, second, .. } => {
                first.collect_panes_mut(list);
                second.collect_panes_mut(list);
            }
        }
    }

    pub fn adjust_ratio_for_target(&mut self, target_id: PaneId, dir: Direction, delta: f32) -> bool {
        match self {
            PaneNode::Leaf(_) => false,
            PaneNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let contains_in_first = first.find_pane(target_id).is_some();
                let contains_in_second = second.find_pane(target_id).is_some();

                let is_matching_axis = match (direction, dir) {
                    (SplitDirection::Horizontal, Direction::Left | Direction::Right) => true,
                    (SplitDirection::Vertical, Direction::Top | Direction::Down) => true,
                    _ => false,
                };

                if is_matching_axis && (contains_in_first || contains_in_second) {
                    let d = match dir {
                        Direction::Right | Direction::Down => {
                            if contains_in_first { delta } else { -delta }
                        }
                        Direction::Left | Direction::Top => {
                            if contains_in_first { -delta } else { delta }
                        }
                    };
                    *ratio = (*ratio + d).clamp(0.1, 0.9);
                    return true;
                }

                if contains_in_first {
                    first.adjust_ratio_for_target(target_id, dir, delta)
                } else if contains_in_second {
                    second.adjust_ratio_for_target(target_id, dir, delta)
                } else {
                    false
                }
            }
        }
    }

    pub fn set_split_ratio_by_path(&mut self, path: &[usize], new_ratio: f32) -> bool {
        match self {
            PaneNode::Leaf(_) => false,
            PaneNode::Split { ratio, first, second, .. } => {
                if path.is_empty() {
                    *ratio = new_ratio.clamp(0.1, 0.9);
                    true
                } else {
                    let next = path[0];
                    let rest = &path[1..];
                    if next == 0 {
                        first.set_split_ratio_by_path(rest, new_ratio)
                    } else if next == 1 {
                        second.set_split_ratio_by_path(rest, new_ratio)
                    } else {
                        false
                    }
                }
            }
        }
    }

    pub fn to_persisted(&self) -> crate::session_manager::PersistedPaneNode {
        match self {
            PaneNode::Leaf(pane) => crate::session_manager::PersistedPaneNode::Leaf(crate::session_manager::PersistedPane {
                id: pane.id,
                title: pane.title.clone(),
                custom_title: pane.custom_title.clone(),
                cwd: pane.cwd.as_ref().map(|c| c.to_string_lossy().to_string()),
            }),
            PaneNode::Split { direction, ratio, first, second } => crate::session_manager::PersistedPaneNode::Split {
                direction: match direction {
                    SplitDirection::Horizontal => "Horizontal".to_string(),
                    SplitDirection::Vertical => "Vertical".to_string(),
                },
                ratio: *ratio,
                first: Box::new(first.to_persisted()),
                second: Box::new(second.to_persisted()),
            },
        }
    }

    pub fn restore_from_persisted<F>(persisted: &crate::session_manager::PersistedPaneNode, spawn_pane: &mut F) -> Self
    where
        F: FnMut(Option<&std::path::Path>, Option<String>) -> TerminalPane,
    {
        match persisted {
            crate::session_manager::PersistedPaneNode::Leaf(p) => {
                let cwd = p.cwd.as_deref().map(std::path::Path::new);
                let title = p.custom_title.clone().or(Some(p.title.clone()));
                let pane = spawn_pane(cwd, title);
                PaneNode::Leaf(pane)
            }
            crate::session_manager::PersistedPaneNode::Split { direction, ratio, first, second } => {
                let dir = if direction == "Vertical" {
                    SplitDirection::Vertical
                } else {
                    SplitDirection::Horizontal
                };
                let first_node = Self::restore_from_persisted(first, spawn_pane);
                let second_node = Self::restore_from_persisted(second, spawn_pane);
                PaneNode::Split {
                    direction: dir,
                    ratio: *ratio,
                    first: Box::new(first_node),
                    second: Box::new(second_node),
                }
            }
        }
    }

    pub fn for_each_terminal<F: FnMut(&Arc<TerminalState>)>(&self, f: &mut F) {
        match self {
            PaneNode::Leaf(pane) => {
                if let Some(ref t) = pane.terminal {
                    f(t);
                }
            }
            PaneNode::Split { first, second, .. } => {
                first.for_each_terminal(f);
                second.for_each_terminal(f);
            }
        }
    }
}

pub struct PaneTree {
    pub root: PaneNode,
    pub active_pane_id: PaneId,
}

impl PaneTree {
    pub fn new(initial_pane: TerminalPane) -> Self {
        let active_pane_id = initial_pane.id;
        Self {
            root: PaneNode::Leaf(initial_pane),
            active_pane_id,
        }
    }

    pub fn find_pane(&self, id: PaneId) -> Option<&TerminalPane> {
        self.root.find_pane(id)
    }

    pub fn find_pane_mut(&mut self, id: PaneId) -> Option<&mut TerminalPane> {
        self.root.find_pane_mut(id)
    }

    pub fn active_pane(&self) -> Option<&TerminalPane> {
        self.find_pane(self.active_pane_id)
    }

    pub fn active_pane_mut(&mut self) -> Option<&mut TerminalPane> {
        self.find_pane_mut(self.active_pane_id)
    }

    pub fn all_panes(&self) -> Vec<&TerminalPane> {
        let mut list = Vec::new();
        self.root.collect_panes(&mut list);
        list
    }

    pub fn all_panes_mut(&mut self) -> Vec<&mut TerminalPane> {
        let mut list = Vec::new();
        self.root.collect_panes_mut(&mut list);
        list
    }

    pub fn pane_count(&self) -> usize {
        self.all_panes().len()
    }

    pub fn for_each_terminal<F: FnMut(&Arc<TerminalState>)>(&self, mut f: F) {
        self.root.for_each_terminal(&mut f);
    }

    pub fn split_active_pane(&mut self, new_pane: TerminalPane, direction: Direction) {
        let new_id = new_pane.id;
        let root = std::mem::replace(
            &mut self.root,
            PaneNode::Leaf(TerminalPane {
                id: 0,
                terminal: None,
                title: String::new(),
                custom_title: None,
                cwd: None,
                git_status: None,
                git_checked_cwd: None,
                git_last_poll: None,
                last_duration_ms: None,
                last_exit_code: None,
                last_bounds: None,
            }),
        );
        self.root = root.split_at_target(self.active_pane_id, new_pane, direction);
        self.active_pane_id = new_id;
    }

    pub fn close_pane(&mut self, target_id: PaneId) -> bool {
        let root = std::mem::replace(
            &mut self.root,
            PaneNode::Leaf(TerminalPane {
                id: 0,
                terminal: None,
                title: String::new(),
                custom_title: None,
                cwd: None,
                git_status: None,
                git_checked_cwd: None,
                git_last_poll: None,
                last_duration_ms: None,
                last_exit_code: None,
                last_bounds: None,
            }),
        );
        match root.close_at_target(target_id) {
            Some(new_root) => {
                self.root = new_root;
                if self.active_pane_id == target_id {
                    if let Some(first_pane) = self.all_panes().first() {
                        self.active_pane_id = first_pane.id;
                    }
                }
                true
            }
            None => false,
        }
    }

    pub fn focus_direction(&mut self, direction: Direction) -> Option<PaneId> {
        let active = self.find_pane(self.active_pane_id)?;
        let active_bounds = active.last_bounds?;
        let ax = (active_bounds.origin.x + active_bounds.size.width / 2.0).to_f64() as f32;
        let ay = (active_bounds.origin.y + active_bounds.size.height / 2.0).to_f64() as f32;

        let mut best: Option<(PaneId, f32)> = None;

        for pane in self.all_panes() {
            if pane.id == self.active_pane_id {
                continue;
            }
            let Some(b) = pane.last_bounds else { continue };
            let px = (b.origin.x + b.size.width / 2.0).to_f64() as f32;
            let py = (b.origin.y + b.size.height / 2.0).to_f64() as f32;

            let in_dir = match direction {
                Direction::Left => px < ax - 1.0,
                Direction::Right => px > ax + 1.0,
                Direction::Top => py < ay - 1.0,
                Direction::Down => py > ay + 1.0,
            };

            if in_dir {
                let dist = (px - ax).powi(2) + (py - ay).powi(2);
                if best.map_or(true, |(_, min)| dist < min) {
                    best = Some((pane.id, dist));
                }
            }
        }

        if let Some((target_id, _)) = best {
            self.active_pane_id = target_id;
            Some(target_id)
        } else {
            None
        }
    }

    pub fn resize_active(&mut self, direction: Direction, delta: f32) -> bool {
        self.root.adjust_ratio_for_target(self.active_pane_id, direction, delta)
    }

    pub fn set_split_ratio_by_path(&mut self, path: &[usize], new_ratio: f32) -> bool {
        self.root.set_split_ratio_by_path(path, new_ratio)
    }

    pub fn update_layout_bounds(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.root.update_bounds(x, y, w, h);
    }
}

impl PaneNode {
    pub fn update_bounds(&mut self, x: f32, y: f32, w: f32, h: f32) {
        match self {
            PaneNode::Leaf(pane) => {
                pane.last_bounds = Some(Bounds {
                    origin: gpui::Point { x: gpui::px(x), y: gpui::px(y) },
                    size: gpui::Size { width: gpui::px(w), height: gpui::px(h) },
                });
            }
            PaneNode::Split { direction, ratio, first, second } => match direction {
                SplitDirection::Horizontal => {
                    let w1 = w * *ratio;
                    let w2 = w * (1.0 - *ratio);
                    first.update_bounds(x, y, w1, h);
                    second.update_bounds(x + w1, y, w2, h);
                }
                SplitDirection::Vertical => {
                    let h1 = h * *ratio;
                    let h2 = h * (1.0 - *ratio);
                    first.update_bounds(x, y, w, h1);
                    second.update_bounds(x, y + h1, w, h2);
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_pane(id: usize) -> TerminalPane {
        TerminalPane {
            id,
            terminal: None,
            title: format!("Pane {}", id),
            custom_title: None,
            cwd: None,
            git_status: None,
            git_checked_cwd: None,
            git_last_poll: None,
            last_duration_ms: None,
            last_exit_code: None,
            last_bounds: None,
        }
    }

    #[test]
    fn test_set_split_ratio_by_path() {
        let mut tree = PaneTree::new(dummy_pane(1));
        tree.split_active_pane(dummy_pane(2), Direction::Right);
        assert_eq!(tree.pane_count(), 2);

        // Root split ratio adjustment
        assert!(tree.set_split_ratio_by_path(&[], 0.7));
        if let PaneNode::Split { ratio, .. } = &tree.root {
            assert!((ratio - 0.7).abs() < 1e-4);
        } else {
            panic!("Expected split root");
        }

        // Sub-split
        tree.split_active_pane(dummy_pane(3), Direction::Down);
        assert_eq!(tree.pane_count(), 3);
        assert!(tree.set_split_ratio_by_path(&[1], 0.35));
    }
}
