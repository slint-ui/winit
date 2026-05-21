use std::error::Error;

use tracing::{error, info};
use winit::application::ApplicationHandler;
use winit::data_transfer::TypeHint;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, AsyncRequestSerial, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

#[path = "util/fill.rs"]
mod fill;
#[path = "util/tracing.rs"]
mod tracing;

fn main() -> Result<(), Box<dyn Error>> {
    tracing::init();

    let event_loop = EventLoop::new()?;

    let app = Application::new();
    Ok(event_loop.run_app(app)?)
}

/// Application state and event handling.
#[derive(Debug, Default)]
struct Application {
    window: Option<Box<dyn Window>>,
    last_dnd_fetch: Option<(AsyncRequestSerial, bool)>,
}

impl Application {
    fn new() -> Self {
        Self::default()
    }
}

impl ApplicationHandler for Application {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let window_attributes =
            WindowAttributes::default().with_title("Drag and drop files on me!");
        self.window = Some(event_loop.create_window(window_attributes).unwrap());
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::DragLeft { .. } => {
                info!("{event:?}");
                self.last_dnd_fetch = None;
            },
            WindowEvent::DragPosition { .. } => {
                info!("{event:?}");
            },
            WindowEvent::DragDropped { .. } => {
                info!("{event:?}");

                if let Some((last_fetch_serial, received)) = &self.last_dnd_fetch
                    && let Some(window) = self.window.as_ref()
                {
                    if *received {
                        let mut data = window.data_transfer_result(*last_fetch_serial).unwrap();
                        let uris = data.try_as_uris().unwrap();
                        info!("{uris:#?}");
                    } else {
                        info!("Never received");
                    }
                }

                self.last_dnd_fetch = None;
            },
            WindowEvent::DataTransferResult { serial, .. } => {
                info!("{event:?}");

                if let Some((last_fetch_serial, received)) = &mut self.last_dnd_fetch
                    && serial == *last_fetch_serial
                {
                    *received = true;
                }

                if let Some(window) = self.window.as_ref() {
                    let mut data = window.data_transfer_result(serial).unwrap();
                    let uris = data.try_as_uris().unwrap();
                    info!("{uris:#?}");
                }
            },
            WindowEvent::DragEntered { id } => {
                info!("{event:?}");

                let type_ = TypeHint::UriList;

                if let Some(window) = self.window.as_ref() {
                    let data_transfer = match window.data_transfer(id) {
                        Ok(dt) => dt,
                        Err(e) => {
                            error!("{e}");
                            return;
                        },
                    };

                    info!(
                        "Types: {:#?}",
                        data_transfer
                            .available_types()
                            .into_iter()
                            .filter_map(|ty| ty.hint())
                            .collect::<Vec<_>>()
                    );

                    if !data_transfer.has_type(&type_) {
                        info!("Cannot drop (cannot interpret input as URI list)");
                        return;
                    }

                    window.accept_drag_type(id, &type_).unwrap();

                    self.last_dnd_fetch =
                        window.fetch_data_transfer(id, &type_).ok().map(|serial| (serial, false));
                } else {
                    error!("No window!");
                }
            },
            WindowEvent::RedrawRequested => {
                let window = self.window.as_ref().unwrap();
                window.pre_present_notify();
                fill::fill_window(window.as_ref());
            },
            WindowEvent::CloseRequested => {
                event_loop.exit();
            },
            _ => {},
        }
    }
}
