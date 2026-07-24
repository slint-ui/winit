//! The [`Popup`] trait and associated types
//! The anchoring system is inspired from the wayland protocol
//! ([XDG Positioner](https://wayland.app/protocols/xdg-shell#xdg_positioner))
//! and implemented for the following platforms:
//! - Linux Wayland
//! - Windows
//! - MacOs

use core::fmt;

use dpi::{Position, Size};

use crate::as_any::AsAny;

/// Anchor rect within the parent surface
/// See: https://wayland.app/protocols/xdg-shell#xdg_positioner:request:set_anchor_rect
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[non_exhaustive]
pub enum PopupAnchor {
    #[default]
    None,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    BottomLeft,
    TopRight,
    BottomRight,
}

/// Defines in what direction a surface should be positioned
/// See: https://wayland.app/protocols/xdg-shell#xdg_positioner:request:set_gravity
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[non_exhaustive]
pub enum PopupGravity {
    #[default]
    None,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    BottomLeft,
    TopRight,
    BottomRight,
}

bitflags::bitflags! {
    /// Specify how the window should be positioned if the originally intended position caused the
    /// surface to be constrained See: https://wayland.app/protocols/xdg-shell#xdg_positioner:request:set_constraint_adjustment
    /// For all other platforms than wayland the behaviour is simulated on the winit side
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct PopupConstraintAdjustment: u32 {
        const SLIDE_X = 1 << 0;
        const SLIDE_Y = 1 << 1;
        const FLIP_X = 1 << 2;
        const FLIP_Y = 1 << 3;
        const RESIZE_X = 1 << 4;
        const RESIZE_Y = 1 << 5;
    }
}

/// Represents a popup window
pub trait Popup: AsAny + Send + Sync + fmt::Debug {
    fn anchor_rect(&self) -> Option<(impl Into<Position>, impl Into<Size>)>;

    /// Sets the anchor edge of the parent surface the popup is positioned relative to.
    ///
    /// See [`PopupAnchor`] for the available edges and corners.
    fn set_anchor(&self, anchor: PopupAnchor);

    /// Sets the anchor rectangle within the parent surface the popup is positioned relative to.
    ///
    /// `position` is the top-left corner of the rectangle relative to the parent window's content
    /// area, and `size` its dimensions.
    fn set_anchor_rect(&self, position: impl Into<Position>, size: impl Into<Size>);

    /// Sets how the compositor should reposition the popup when it would be constrained by screen
    /// edges.
    ///
    /// See [`PopupConstraintAdjustment`] for the available adjustment flags.
    fn set_constraint_adjustment(&self, constraint_adjustment: PopupConstraintAdjustment);

    /// Sets the direction the popup surface extends from the anchor point.
    ///
    /// See [`PopupGravity`] for the available directions.
    fn set_gravity(&self, gravity: PopupGravity);

    /// Set the popup position relative to the anchor rect
    fn set_positioner_offset(&self, position: impl Into<Position>);
}
