pub mod never_return;
pub mod pump_events;
pub mod register;
pub mod run_on_demand;

use std::fmt::{self, Debug};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use dpi::PhysicalPosition;
use rwh_06::{DisplayHandle, HandleError, HasDisplayHandle};

use crate::Instant;
use crate::as_any::AsAny;
use crate::cursor::{CustomCursor, CustomCursorSource};
use crate::data_transfer::{DataTransfer, DataTransferId, DataTransferSend, TransferType};
use crate::error::{NotSupportedError, RequestError};
use crate::icon::Icon;
use crate::monitor::MonitorHandle;
use crate::window::{Theme, Window, WindowAttributes, WindowId};

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
    ) -> Result<AsyncRequestSerial, RequestError> {
        let _ = id;
        let _ = type_;
        Err(RequestError::NotSupported(NotSupportedError::new(
            DATA_TRANSFER_UNSUPPORTED_ERROR_MESSAGE,
        )))
    }

    /// Get a [data transfer](DataTransfer) by its ID.
    ///
    /// If the ID is invalid (e.g. if the lifetime of the data transfer has expired), this will
    /// return an error.
    fn data_transfer(&self, id: DataTransferId) -> Result<Box<dyn DataTransfer>, RequestError> {
        let _ = id;
        Err(RequestError::NotSupported(NotSupportedError::new(
            DATA_TRANSFER_UNSUPPORTED_ERROR_MESSAGE,
        )))
    }

    /// Set a given set of `DndAction`s as the valid actions for the given [`DataTransferId`],
    /// if the transfer ID is from an incoming drag-and-drop operation.
    ///
    /// This allows the OS/compositor to display the correct UI, indicating that the dragged data
    /// can be dropped. If the data transfer does not exist or is not from a drag-and-drop
    /// operation, will return an error.
    ///
    /// The set of actions is expected to be ordered by preference.
    fn set_actions(&self, id: DataTransferId, actions: &[DndAction]) -> Result<(), RequestError> {
        let _ = id;
        let _ = actions;
        Err(RequestError::NotSupported(NotSupportedError::new(
            DATA_TRANSFER_UNSUPPORTED_ERROR_MESSAGE,
        )))
    }

    /// If the [`DataTransferId`] corresponds to an incoming drag-and-drop operation, return the
    /// set of available [`DndAction`]s for that operation.
    fn valid_actions(&self, id: DataTransferId) -> Result<Vec<DndAction>, RequestError> {
        let _ = id;
        Err(RequestError::NotSupported(NotSupportedError::new(
            DATA_TRANSFER_UNSUPPORTED_ERROR_MESSAGE,
        )))
    }

    /// Initiate a new drag-and-drop operation.
    ///
    /// See [`DataTransferSendBuilder`](crate::data_transfer::DataTransferSendBuilder) for how to
    /// create a new cross-platform data transfer, or see the platform-specific implementation
    /// of [`DataTransferSend`].
    ///
    /// ### Arguments
    ///
    /// - `source` - The ID of the window that initiated the drag operation.
    /// - `send_data` - The data provided by this drag operation. See
    ///   [`DataTransferSendBuilder`](crate::data_transfer::DataTransferSendBuilder).
    /// - `actions` - The set of valid actions for this drag operation. See
    ///   [`DndAction`](crate::data_transfer::DndAction). On Wayland, this is expected to be ordered
    ///   by preference.
    /// - `icon` - The icon to show while dragging.
    ///
    /// Some platforms have a more-expressive way of setting the visual component of a drag
    /// operation. For those platforms, consider using the platform-specific implementation of
    /// [`DataTransferSend`] for `send_data` and set this field to `None`.
    ///
    /// ### Returns
    ///
    /// A `DataTransferId` that can be used to identify this drag operation when events are received
    /// such as `OutgoingDragEnded`. Note that if a drag-and-drop operation starts and ends from the
    /// same window or process, the transfer ID that is reported by `DragEntered` is _not_
    /// guaranteed to be the same as the one returned by this method. On most platforms, it
    /// should be safe to make the assumption that if an outgoing drag is still in progress,
    /// then any incoming drag is the same operation. However, that assumption is not encoded in
    /// winit to avoid misleading users.
    fn start_drag(
        &self,
        source: WindowId,
        send_data: Box<dyn DataTransferSend>,
        actions: &[DndAction],
        icon: Option<DragIcon>,
    ) -> Result<DataTransferId, RequestError> {
        let _ = source;
        let _ = send_data;
        let _ = actions;
        let _ = icon;
        Err(RequestError::NotSupported(NotSupportedError::new(
            DATA_TRANSFER_UNSUPPORTED_ERROR_MESSAGE,
        )))
    }
}

const DATA_TRANSFER_UNSUPPORTED_ERROR_MESSAGE: &str = {
    "Cross-application data transfer (e.g. drag-and-drop, clipboard) is unsupported on this \
     platform"
};

impl HasDisplayHandle for dyn ActiveEventLoop + '_ {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.rwh_06_handle().display_handle()
    }
}

impl_dyn_casting!(ActiveEventLoop);

/// Information needed to initiate a new drag operation.
pub struct DragIcon {
    /// The icon to apply to the cursor.
    pub icon: Icon,
    /// An offset applied to the dragged icon. 0,0 means that the top-left point
    /// of the icon will be at the cursor.
    pub offset: PhysicalPosition<i32>,
}

impl From<Icon> for DragIcon {
    fn from(value: Icon) -> Self {
        Self { icon: value, offset: Default::default() }
    }
}

/// The set of available actions for a drag operation.
///
/// This is _not_ a bitset, as on some platforms (e.g. Wayland, macOS) the source and/or destination
/// are expected to provide some kind of order of preference.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DndAction {
    /// Move the dragged item from the source to the destination.
    ///
    /// # Platforms
    ///
    /// - X11
    /// - Wayland
    /// - macOS
    /// - Windows
    Move,
    /// Copy the dragged item from the source to the destination.
    ///
    /// # Platforms
    ///
    /// - X11
    /// - Wayland
    /// - macOS
    /// - Windows
    Copy,
    /// A link is established between the source and the destination.
    ///
    /// # Platforms
    ///
    /// - macOS
    /// - Windows
    /// - X11
    Link,
    /// The user will be prompted for what should be done
    ///
    /// # Platforms
    ///
    /// - Wayland
    Ask,
    /// The source and destination will negotiate the drag operation privately
    ///
    /// # Platforms
    ///
    /// - X11
    /// - macOS
    Private,
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
