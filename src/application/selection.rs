use crate::domain::PackageId;
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClickModifiers {
    pub command: bool,
    pub shift: bool,
}

#[derive(Default)]
pub struct SelectionModel {
    visible: Arc<[PackageId]>,
    selected: HashSet<PackageId>,
    cursor: Option<usize>,
    anchor: Option<usize>,
}

impl SelectionModel {
    pub fn visible(&self) -> &[PackageId] {
        &self.visible
    }

    pub fn selected(&self) -> &HashSet<PackageId> {
        &self.selected
    }

    pub const fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    pub fn set_visible(&mut self, visible: Arc<[PackageId]>) {
        let cursor_id = self
            .cursor
            .and_then(|index| self.visible.get(index))
            .cloned();
        self.visible = visible;
        let visible_ids: HashSet<_> = self.visible.iter().collect();
        self.selected.retain(|id| visible_ids.contains(id));
        self.cursor = cursor_id
            .as_ref()
            .and_then(|id| self.visible.iter().position(|candidate| candidate == id))
            .or_else(|| (!self.visible.is_empty()).then_some(0));
        self.anchor = self
            .anchor
            .map(|index| index.min(self.visible.len().saturating_sub(1)));
        if self.visible.is_empty() {
            self.cursor = None;
            self.anchor = None;
        }
    }

    pub fn clear(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    pub fn select_all(&mut self) {
        self.selected = self.visible.iter().cloned().collect();
        self.cursor = (!self.visible.is_empty()).then_some(0);
        self.anchor = self.cursor;
    }

    pub fn click(&mut self, index: usize, modifiers: ClickModifiers) {
        let Some(id) = self.visible.get(index).cloned() else {
            return;
        };
        self.cursor = Some(index);
        if modifiers.shift {
            let anchor = self.anchor.unwrap_or(index);
            self.anchor = Some(anchor);
            self.select_range(anchor, index);
        } else if modifiers.command {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
            self.anchor = Some(index);
        } else {
            self.selected.clear();
            self.selected.insert(id);
            self.anchor = Some(index);
        }
    }

    pub fn move_cursor(&mut self, delta: isize, extend: bool) -> Option<usize> {
        if self.visible.is_empty() {
            self.cursor = None;
            return None;
        }
        let last = self.visible.len() - 1;
        let previous = self.cursor;
        let next = match previous {
            Some(index) => index.saturating_add_signed(delta).min(last),
            None if delta >= 0 => 0,
            None => last,
        };
        self.cursor = Some(next);
        if extend {
            let anchor = self.anchor.or(previous).unwrap_or(next);
            self.anchor = Some(anchor);
            self.select_range(anchor, next);
        } else {
            self.selected.clear();
            self.selected.insert(self.visible[next].clone());
            self.anchor = Some(next);
        }
        Some(next)
    }

    fn select_range(&mut self, left: usize, right: usize) {
        let range = left.min(right)..=left.max(right);
        self.selected = self.visible[range].iter().cloned().collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::PackageKind;

    fn id(name: &str, kind: PackageKind) -> PackageId {
        PackageId::new(name, kind).unwrap()
    }

    fn visible() -> Vec<PackageId> {
        ["a", "b", "c", "d", "e"]
            .map(|name| id(name, PackageKind::Formula))
            .into()
    }

    #[test]
    fn shift_arrow_expands_and_contracts_from_anchor() {
        let mut selection = SelectionModel::default();
        selection.set_visible(visible().into());
        selection.click(2, ClickModifiers::default());
        selection.move_cursor(1, true);
        selection.move_cursor(1, true);
        assert_eq!(selection.selected.len(), 3);
        selection.move_cursor(-1, true);
        assert_eq!(selection.selected.len(), 2);
        assert!(selection.selected.contains(&id("c", PackageKind::Formula)));
        assert!(selection.selected.contains(&id("d", PackageKind::Formula)));
    }

    #[test]
    fn shift_click_selects_the_anchored_range() {
        let mut selection = SelectionModel::default();
        selection.set_visible(visible().into());
        selection.click(1, ClickModifiers::default());
        selection.click(
            4,
            ClickModifiers {
                command: false,
                shift: true,
            },
        );
        assert_eq!(selection.selected.len(), 4);
        assert!(!selection.selected.contains(&id("a", PackageKind::Formula)));
        assert!(selection.selected.contains(&id("e", PackageKind::Formula)));
    }

    #[test]
    fn formula_and_cask_with_same_name_select_independently() {
        let formula = id("ant", PackageKind::Formula);
        let cask = id("ant", PackageKind::Cask);
        let mut selection = SelectionModel::default();
        selection.set_visible(vec![formula.clone(), cask.clone()].into());
        selection.click(0, ClickModifiers::default());
        selection.click(
            1,
            ClickModifiers {
                command: true,
                shift: false,
            },
        );
        assert!(selection.selected.contains(&formula));
        assert!(selection.selected.contains(&cask));
    }

    #[test]
    fn filtering_removes_stale_ids_and_keeps_cursor_safe() {
        let mut selection = SelectionModel::default();
        selection.set_visible(visible().into());
        selection.click(4, ClickModifiers::default());
        selection.set_visible(vec![id("a", PackageKind::Formula)].into());
        assert_eq!(selection.cursor(), Some(0));
        assert!(selection.selected().is_empty());
        selection.set_visible(Vec::new().into());
        assert_eq!(selection.cursor(), None);
    }
}
