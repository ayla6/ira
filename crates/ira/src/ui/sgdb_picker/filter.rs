use super::super::css::*;
use super::cards::{find_filter_button, tag_author, tag_style};
use super::render::{append_assets, full_rebuild};
use super::types::SgdbPickerCtx;
use adw::prelude::*;

impl SgdbPickerCtx {
    pub(super) fn rebuild_all(&self) {
        let (Some(flow), Some(list_view)) = (self.flow.upgrade(), self.list_view.upgrade())
        else {
            return;
        };
        let assets = self.store.borrow();
        full_rebuild(&flow, &list_view, &assets, self);
        self.rendered.set(assets.len());
    }

    pub(super) fn append_new(&self) {
        let start = self.rendered.get();
        if start >= self.store.borrow().len() {
            return;
        }
        let (Some(flow), Some(list_view)) = (self.flow.upgrade(), self.list_view.upgrade())
        else {
            return;
        };
        let assets = self.store.borrow();
        append_assets(&flow, &list_view, &assets, start, self);
        self.rendered.set(assets.len());
    }

    /// Reposition one author's cards after a filter toggle: dimmed to
    /// the end, visible just before the first dimmed — the grid never
    /// stripes visible/dimmed/visible.
    fn settle_author_cards(&self, author: &str) {
        let (Some(flow), Some(list)) = (self.flow.upgrade(), self.list_view.upgrade())
        else {
            return;
        };
        settle_flow_author(self, &flow, author);
        settle_list_author(self, &list, author);
    }

    /// The eye button's click: filter a shown author, unfilter an
    /// already-filtered one. The state is read live so every one of the
    /// author's cards agrees no matter which card was clicked.
    pub(super) fn toggle_author(&self, author: &str, steam64: &str) {
        if self.steam.user_filtered(author) {
            self.unfilter_author(author);
        } else {
            self.filter_author(author, steam64);
        }
    }

    /// Run `f` over every card and row tag belonging to `author`, in
    /// place — no rebuild, so loaded thumbnails and the scroll position
    /// stay untouched. Shared by the filter and unfilter halves.
    fn for_each_author_card(&self, author: &str, mut f: impl FnMut(&gtk4::Widget)) {
        let (Some(flow), Some(list_view)) = (self.flow.upgrade(), self.list_view.upgrade())
        else {
            return;
        };
        for container in [&flow.upcast::<gtk4::Widget>(), &list_view.upcast()] {
            let mut child = container.first_child();
            while let Some(w) = child {
                child = w.next_sibling();
                if tag_author(&w.widget_name()) == author {
                    f(&w);
                }
            }
        }
    }

    /// Dim one author's art in place: persist the choice, dim exactly
    /// that author's cards and rows where they stand, and flip their eye
    /// buttons to the unfilter state. The art stays visible, just
    /// transparent, and the store keeps its order for later pages.
    fn filter_author(&self, author: &str, steam64: &str) {
        if let Some(persist) = &self.filter_user {
            persist(author.to_string(), steam64.to_string());
        }
        self.for_each_author_card(author, |w| self.dim_card(w));
        self.settle_author_cards(author);
    }

    /// Dim one card-or-row and flip its eye button to the unfilter
    /// state: the mirror of [`Self::undim_card`] on the same widgets.
    fn dim_card(&self, widget: &gtk4::Widget) {
        widget.add_css_class(CSS_SGDB_DIMMED);
        if let Some(btn) = find_filter_button(widget) {
            btn.set_icon_name("eye-open-negative-filled-symbolic");
            btn.set_tooltip_text(Some(&crate::tr!("Show art by this author")));
        }
    }

    /// Show one author's art again: persist the removal, undim exactly
    /// that author's cards and rows, and flip their eye buttons back.
    /// Cards whose style is still filtered keep their dim.
    fn unfilter_author(&self, author: &str) {
        if let Some(persist) = &self.unfilter_user {
            persist(author.to_string());
        }
        self.for_each_author_card(author, |w| self.undim_card(w));
        self.settle_author_cards(author);
    }

    /// Drop one card-or-row's dim when its style is no longer filtered and
    /// flip its eye button back to hide. The author half just left the
    /// filtered-users list, so only the tag's style half still matters.
    fn undim_card(&self, widget: &gtk4::Widget) {
        if !self.steam.style_filtered(tag_style(&widget.widget_name())) {
            widget.remove_css_class(CSS_SGDB_DIMMED);
        }
        if let Some(btn) = find_filter_button(widget) {
            btn.set_icon_name("eye-not-looking-symbolic");
            btn.set_tooltip_text(Some(&crate::tr!("Hide art by this author")));
        }
    }
}

/// Settle one author's flow children: detach, then reinsert dimmed at
/// the end and visible before the first dimmed, all order-preserved.
fn settle_flow_author(ctx: &SgdbPickerCtx, flow: &gtk4::FlowBox, author: &str) {
    let mut mine = Vec::new();
    let mut child = flow.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        if tag_author(&w.widget_name()) == author {
            mine.push(w);
        }
    }
    for w in &mine {
        flow.remove(w);
    }
    let mut pos = first_dimmed_pos(flow, ctx);
    for w in &mine {
        if card_dimmed(ctx, w) {
            flow.append(w);
        } else if let Some(p) = pos {
            flow.insert(w, p);
            pos = Some(p + 1);
        } else {
            flow.append(w);
        }
    }
}

/// Settle one author's list rows, mirroring [`settle_flow_author`].
fn settle_list_author(ctx: &SgdbPickerCtx, list: &gtk4::ListBox, author: &str) {
    let mut mine = Vec::new();
    let mut child = list.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        if tag_author(&w.widget_name()) == author {
            mine.push(w);
        }
    }
    for w in &mine {
        list.remove(w);
    }
    let mut pos = first_dimmed_pos(list, ctx);
    for w in &mine {
        if card_dimmed(ctx, w) {
            list.append(w);
        } else if let Some(p) = pos {
            list.insert(w, p);
            pos = Some(p + 1);
        } else {
            list.append(w);
        }
    }
}

fn card_dimmed(ctx: &SgdbPickerCtx, widget: &gtk4::Widget) -> bool {
    let name = widget.widget_name();
    ctx.steam
        .asset_filtered(tag_author(&name), tag_style(&name))
}

fn first_dimmed_pos(container: &impl IsA<gtk4::Widget>, ctx: &SgdbPickerCtx) -> Option<i32> {
    let mut index = 0;
    let mut child = container.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        if card_dimmed(ctx, &w) {
            return Some(index);
        }
        index += 1;
    }
    None
}

pub(super) fn detach_dimmed(
    ctx: &SgdbPickerCtx,
    container: &impl IsA<gtk4::Widget>,
) -> Vec<gtk4::Widget> {
    let mut out = Vec::new();
    let mut child = container.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        if card_dimmed(ctx, &w) {
            out.push(w);
        }
    }
    out
}
