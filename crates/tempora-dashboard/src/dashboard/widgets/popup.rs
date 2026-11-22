use eframe::egui::{self, Align, Context, Id, Key, Order, Rect, Ui, pos2};

use super::{menu, popover};

pub(in crate::dashboard) struct PopupOutput<R> {
    pub inner: R,
    /// Esc, or a press outside both the popover and its anchor.
    pub dismissed: bool,
}

/// Shows a popover in the foreground layer under `anchor`, aligned to its left or right
/// edge and kept on screen.
pub(in crate::dashboard) fn anchored_popover<R>(
    ctx: &Context,
    id: Id,
    anchor: Rect,
    align: Align,
    content: impl FnOnce(&mut Ui) -> R,
) -> PopupOutput<R> {
    layer(ctx, id, anchor, align, |ui| popover(ui, content).inner)
}

/// An anchored menu (items height 34, destructive items in the bad color).
pub(in crate::dashboard) fn anchored_menu<R>(
    ctx: &Context,
    id: Id,
    anchor: Rect,
    align: Align,
    content: impl FnOnce(&mut Ui) -> R,
) -> PopupOutput<R> {
    layer(ctx, id, anchor, align, |ui| menu(ui, content).inner)
}

fn layer<R>(
    ctx: &Context,
    id: Id,
    anchor: Rect,
    align: Align,
    content: impl FnOnce(&mut Ui) -> R,
) -> PopupOutput<R> {
    const OFFSET: f32 = 6.0;
    let (pivot, position) = if align == Align::Max {
        (
            egui::Align2::RIGHT_TOP,
            pos2(anchor.right(), anchor.bottom() + OFFSET),
        )
    } else {
        (
            egui::Align2::LEFT_TOP,
            pos2(anchor.left(), anchor.bottom() + OFFSET),
        )
    };
    let shown = egui::Area::new(id)
        .order(Order::Foreground)
        .pivot(pivot)
        .fixed_pos(position)
        .constrain(true)
        .show(ctx, content);
    let area = shown.response.rect;
    let dismissed = ctx.input(|input| {
        input.key_pressed(Key::Escape)
            || (input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| !area.contains(pos) && !anchor.contains(pos)))
    });
    PopupOutput {
        inner: shown.inner,
        dismissed,
    }
}
