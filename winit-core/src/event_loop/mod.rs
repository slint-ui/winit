pub mod never_return;
pub mod pump_events;
pub mod register;
pub mod run_on_demand;

use std::fmt::{self, Debug};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rwh_06::{DisplayHandle, HandleError, HasDisplayHandle};

use crate::Instant;
use crate::as_any::AsAny;
use crate::cursor::{CustomCursor, CustomCursorSource};
use crate::data_transfer::{
    DataTransfer, DataTransferId, NewDataTransfer, TransferType, TypedData,
};
use crate::error::{NotSupportedError, RequestError};
use crate::monitor::MonitorHandle;
use crate::window::{Theme, Window, WindowAttributes};

/// An operation was attempted on a data transfer ID, but that ID was invalid.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct UnknownDataTransfer(pub DataTransferId);

impl fmt::Display for UnknownDataTransfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = self.0.into_raw();
        write!(f, "Unknown data transfer with ID {id}")
    }
}

impl std::error::Error for UnknownDataTransfer {}

pub trait ActiveEventLoop: AsAny + fmt::Debug {
    /// Creates an [`EventLoopProxy`] that can be used to dispatch user events
    /// to the main event loop, possibly from another thread.
    fn create_proxy(&self) -> EventLoopProxy;

    /// Create the window.
    ///
    /// Possible causes of error include denied permission, incompatible system, and lack of memory.
    ///
    /// ## Platform-specific
    ///
    /// - **Web:** The window is created but not inserted into the Web page automatically. Please
    ///   see the Web platform module for more information.
    fn create_window(
        &self,
        window_attributes: WindowAttributes,
    ) -> Result<Box<dyn Window>, RequestError>;

    /// Create custom cursor.
    ///
    /// ## Platform-specific
    ///
    /// **iOS / Android / Orbital:** Unsupported.
    fn create_custom_cursor(
        &self,
        custom_cursor: CustomCursorSource,
    ) -> Result<CustomCursor, RequestError>;

    /// Returns the list of all the monitors available on the system.
    ///
    /// ## Platform-specific
    ///
    /// **Web:** Only returns the current monitor without `detailed monitor permissions`.
    fn available_monitors(&self) -> Box<dyn Iterator<Item = MonitorHandle>>;

    /// Returns the primary monitor of the system.
    ///
    /// Returns `None` if it can't identify any monitor as a primary one.
    ///
    /// ## Platform-specific
    ///
    /// - **Wayland:** Always returns `None`.
    /// - **Web:** Always returns `None` without `detailed monitor permissions`.
    fn primary_monitor(&self) -> Option<MonitorHandle>;

    /// Change if or when [`DeviceEvent`]s are captured.
    ///
    /// Since the [`DeviceEvent`] capture can lead to high CPU usage for unfocused windows, winit
    /// will ignore them by default for unfocused windows on Linux/BSD. This method allows changing
    /// this at runtime to explicitly capture them again.
    ///
    /// ## Platform-specific
    ///
    /// - **Wayland / macOS / iOS / Android / Orbital:** Unsupported.
    ///
    /// [`DeviceEvent`]: crate::event::DeviceEvent
    fn listen_device_events(&self, allowed: DeviceEvents);

    /// Returns the current system theme.
    ///
    /// Returns `None` if it cannot be determined on the current platform.
    ///
    /// ## Platform-specific
    ///
    /// - **iOS / Android / Wayland / x11 / Orbital:** Unsupported.
    fn system_theme(&self) -> Option<Theme>;

    /// Sets the [`ControlFlow`].
    fn set_control_flow(&self, control_flow: ControlFlow);

    /// Gets the current [`ControlFlow`].
    fn control_flow(&self) -> ControlFlow;

    /// Stop the event loop.
    ///
    /// ## Platform-specific
    ///
    /// ### iOS
    ///
    /// It is not possible to programmatically exit/quit an application on iOS, so this function is
    /// a no-op there. See also [this technical Q&A][qa1561].
    ///
    /// [qa1561]: https://developer.apple.com/library/archive/qa/qa1561/_index.html
    fn exit(&self);

    /// Returns whether the [`ActiveEventLoop`] is about to stop.
    ///
    /// Set by [`exit()`][Self::exit].
    fn exiting(&self) -> bool;

    /// Gets a persistent reference to the underlying platform display.
    ///
    /// See the [`OwnedDisplayHandle`] type for more information.
    fn owned_display_handle(&self) -> OwnedDisplayHandle;

    /// Get the raw-window-handle handle.
    fn rwh_06_handle(&self) -> &dyn HasDisplayHandle;

    /// Request to fetch a type from a [data transfer](crate::data_transfer::DataTransfer).
    ///
    /// This may be called multiple times on the same [`DataTransferId`] with different types.
    fn fetch_data_transfer(
        &self,
        id: DataTransferId,
        type_: &dyn TransferType,
    ) -> Result<Box<dyn TypedData>, RequestError> {
        let _ = id;
        let _ = type_;
        Err(RequestError::NotSupported(NotSupportedError::new(
            "Cross-application data transfer (e.g. drag-and-drop, clipboard) is unsupported on \
             this platform",
        )))
    }

    /// Get a [data transfer](DataTransfer) by its ID.
    ///
    /// If the ID is invalid (e.g. if the lifetime of the data transfer has expired), this will
    /// return an error.
    fn data_transfer(&self, id: DataTransferId) -> Result<Box<dyn DataTransfer>, RequestError> {
        let _ = id;
        Err(RequestError::NotSupported(NotSupportedError::new(
            "Cross-application data transfer (e.g. drag-and-drop, clipboard) is unsupported on \
             this platform",
        )))
    }

    /// Set a given `DndActionMask` as the valid actions for the given [`DataTransferId`],
    /// presuming that the transfer ID is from a drag-and-drop operation.
    ///
    /// This allows the OS/compositor to display the correct UI, indicating that the dragged data
    /// can be dropped.
    fn set_valid_actions(
        &self,
        id: DataTransferId,
        actions: &dyn DndActionMask,
    ) -> Result<(), UnknownDataTransfer> {
        let _ = actions;
        Err(UnknownDataTransfer(id))
    }

    fn start_drag(
        &self,
        data_transfer: Box<dyn NewDataTransfer>,
    ) -> Result<DataTransferId, RequestError> {
        let _ = data_transfer;
        Err(RequestError::NotSupported(NotSupportedError::new(
            "Cross-application data transfer (e.g. drag-and-drop, clipboard) is unsupported on \
             this platform",
        )))
    }
}

impl HasDisplayHandle for dyn ActiveEventLoop + '_ {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.rwh_06_handle().display_handle()
    }
}

impl_dyn_casting!(ActiveEventLoop);

// Inspired by https://developer.mozilla.org/en-US/docs/Web/API/DataTransfer/dropEffect
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DndActions {
    /// A specific set of operations.
    Flags { move_: bool, copy: bool, link: bool },
    /// All actions, including platform-specific ones not represented in `Self::Flags`.
    All,
}

impl DndActions {
    pub const fn copy(&self) -> bool {
        match *self {
            DndActions::Flags { copy, .. } => copy,
            DndActions::All => true,
        }
    }

    pub const fn move_(&self) -> bool {
        match *self {
            DndActions::Flags { move_, .. } => move_,
            DndActions::All => true,
        }
    }

    pub const fn link(&self) -> bool {
        match *self {
            DndActions::Flags { link, .. } => link,
            DndActions::All => true,
        }
    }

    pub const fn all() -> Self {
        Self::All
    }

    pub const fn none() -> Self {
        Self::Flags { move_: false, copy: false, link: false }
    }

    pub const fn any(&self) -> bool {
        match *self {
            Self::All => true,
            Self::Flags { move_, copy, link } => move_ || copy || link,
        }
    }

    pub const fn is_empty(&self) -> bool {
        !self.any()
    }

    pub const fn intersects(&self, other: &Self) -> bool {
        self.intersection(other).any()
    }

    pub const fn intersection(&self, other: &Self) -> Self {
        match (*self, *other) {
            (Self::All, other) | (other, Self::All) => other,
            (
                Self::Flags { move_: this_move, copy: this_copy, link: this_link },
                Self::Flags { move_: other_move, copy: other_copy, link: other_link },
            ) => Self::Flags {
                move_: this_move && other_move,
                copy: this_copy && other_copy,
                link: this_link && other_link,
            },
        }
    }
}

pub trait DndActionMask: AsAny + fmt::Debug {
    fn hint(&self) -> DndActions;
    fn intersection(&self, other: &dyn DndActionMask) -> Box<dyn DndActionMask>;
    fn is_empty(&self) -> bool;

    fn intersects(&self, other: &dyn DndActionMask) -> bool {
        !self.intersection(other).is_empty()
    }
}

impl_dyn_casting!(DndActionMask);

impl DndActionMask for DndActions {
    fn hint(&self) -> DndActions {
        *self
    }

    fn intersects(&self, other: &dyn DndActionMask) -> bool {
        self.intersects(&other.hint())
    }

    fn intersection(&self, other: &dyn DndActionMask) -> Box<dyn DndActionMask> {
        Box::new(self.intersection(&other.hint()))
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

/// Control the [`ActiveEventLoop`], possibly from a different thread, without referencing it
/// directly.
#[derive(Clone, Debug)]
pub struct EventLoopProxy {
    pub(crate) proxy: Arc<dyn EventLoopProxyProvider>,
}

impl EventLoopProxy {
    /// Wake up the [`ActiveEventLoop`], resulting in [`ApplicationHandler::proxy_wake_up()`] being
    /// called.
    ///
    /// Calls to this method are coalesced into a single call to [`proxy_wake_up`], see the
    /// documentation on that for details.
    ///
    /// If the event loop is no longer running, this is a no-op.
    ///
    /// [`proxy_wake_up`]: crate::application::ApplicationHandler::proxy_wake_up
    /// [`ApplicationHandler::proxy_wake_up()`]: crate::application::ApplicationHandler::proxy_wake_up
    ///
    /// # Platform-specific
    ///
    /// - **Windows**: The wake-up may be ignored under high contention, see [#3687].
    ///
    /// [#3687]: https://github.com/rust-windowing/winit/pull/3687
    pub fn wake_up(&self) {
        self.proxy.wake_up();
    }

    pub fn new(proxy: Arc<dyn EventLoopProxyProvider>) -> Self {
        Self { proxy }
    }
}

pub trait EventLoopProxyProvider: Send + Sync + Debug {
    /// See [`EventLoopProxy::wake_up`] for details.
    fn wake_up(&self);
}

/// A proxy for the underlying display handle.
///
/// The purpose of this type is to provide a cheaply cloneable handle to the underlying
/// display handle. This is often used by graphics APIs to connect to the underlying APIs.
/// It is difficult to keep a handle to the underlying event loop type or the [`ActiveEventLoop`]
/// type. In contrast, this type involves no lifetimes and can be persisted for as long as
/// needed.
///
/// For all platforms, this is one of the following:
///
/// - A zero-sized type that is likely optimized out.
/// - A reference-counted pointer to the underlying type.
#[derive(Clone)]
pub struct OwnedDisplayHandle {
    pub(crate) handle: Arc<dyn HasDisplayHandle + Send + Sync>,
}

impl OwnedDisplayHandle {
    pub fn new(handle: Arc<dyn HasDisplayHandle + Send + Sync>) -> Self {
        Self { handle }
    }
}

impl HasDisplayHandle for OwnedDisplayHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.handle.display_handle()
    }
}

impl fmt::Debug for OwnedDisplayHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedDisplayHandle").finish_non_exhaustive()
    }
}

impl PartialEq for OwnedDisplayHandle {
    fn eq(&self, other: &Self) -> bool {
        match (self.display_handle(), other.display_handle()) {
            (Ok(lhs), Ok(rhs)) => lhs == rhs,
            _ => false,
        }
    }
}

impl Eq for OwnedDisplayHandle {}

/// Set through [`ActiveEventLoop::set_control_flow()`].
///
/// Indicates the desired behavior of the event loop after [`about_to_wait`] is called.
///
/// Defaults to [`Wait`].
///
/// [`Wait`]: Self::Wait
/// [`about_to_wait`]: crate::application::ApplicationHandler::about_to_wait
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum ControlFlow {
    /// When the current loop iteration finishes, immediately begin a new iteration regardless of
    /// whether or not new events are available to process.
    Poll,

    /// When the current loop iteration finishes, suspend the thread until another event arrives.
    #[default]
    Wait,

    /// When the current loop iteration finishes, suspend the thread until either another event
    /// arrives or the given time is reached.
    ///
    /// Useful for implementing efficient timers. Applications which want to render at the
    /// display's native refresh rate should instead use [`Poll`] and the VSync functionality
    /// of a graphics API to reduce odds of missed frames.
    ///
    /// [`Poll`]: Self::Poll
    WaitUntil(Instant),
}

impl ControlFlow {
    /// Creates a [`ControlFlow`] that waits until a timeout has expired.
    ///
    /// In most cases, this is set to [`WaitUntil`]. However, if the timeout overflows, it is
    /// instead set to [`Wait`].
    ///
    /// [`WaitUntil`]: Self::WaitUntil
    /// [`Wait`]: Self::Wait
    pub fn wait_duration(timeout: Duration) -> Self {
        match Instant::now().checked_add(timeout) {
            Some(instant) => Self::WaitUntil(instant),
            None => Self::Wait,
        }
    }
}

/// Control when device events are captured.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DeviceEvents {
    /// Report device events regardless of window focus.
    Always,
    /// Only capture device events while the window is focused.
    #[default]
    WhenFocused,
    /// Never capture device events.
    Never,
}

/// A unique identifier of the winit's async request.
///
/// This could be used to identify the async request once it's done
/// and a specific action must be taken.
///
/// One of the handling scenarios could be to maintain a working list
/// containing [`AsyncRequestSerial`] and some closure associated with it.
/// Then once event is arriving the working list is being traversed and a job
/// executed and removed from the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AsyncRequestSerial {
    serial: usize,
}

impl AsyncRequestSerial {
    pub fn get() -> Self {
        static CURRENT_SERIAL: AtomicUsize = AtomicUsize::new(0);
        // NOTE: We rely on wrap around here, while the user may just request
        // in the loop usize::MAX times that's issue is considered on them.
        let serial = CURRENT_SERIAL.fetch_add(1, Ordering::Relaxed);
        Self { serial }
    }
}
