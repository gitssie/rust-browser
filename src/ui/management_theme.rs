use super::*;

pub(super) fn management_add_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Button {
    Button::new(id).primary().icon(IconName::Plus).label(label)
}

pub(super) fn management_row_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> Button {
    Button::new(id).outline().small().label(label)
}
