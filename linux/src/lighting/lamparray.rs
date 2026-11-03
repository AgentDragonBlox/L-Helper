//! Windows Dynamic Lighting has no Linux counterpart: frames always go to the keyboard's own reports

use super::zone::Color;

pub fn draw(_: &[Color]) -> bool {
    false
}
