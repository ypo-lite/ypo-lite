//! Screens that are still stubs: Skin says what it will hold. (General,
//! Variables and the tray menus have modules of their own.)

use y_pop::settings::Settings;

use crate::ui::prelude::*;
use windows_reactor::*;

pub fn skin(_settings: &Settings) -> View {
    page(
        "Skin",
        "The theme of the popup menus, and what you override on top of it.",
        note("Theme cards, overrides and a live preview come next."),
    )
}
