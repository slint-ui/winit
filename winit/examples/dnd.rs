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

#[derive(Debug)]
struct FetchState {
    serial: AsyncRequestSerial,
    type_: TypeHint,
    received: bool,
}

/// Application state and event handling.
#[derive(Debug, Default)]
struct Application {
    window: Option<Box<dyn Window>>,
    last_dnd_fetch: Option<FetchState>,
}

impl Application {
    fn new() -> Self {
        Self::default()
    }
}

impl ApplicationHandler for Application {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let window_attributes =
            WindowAttributes::default().with_title("Drag and drop files, text or HTML onto me!");
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

                if let Some(state) = &self.last_dnd_fetch {
                    if state.received {
                        let mut data = event_loop.data_transfer_result(state.serial).unwrap();
                        assert_eq!(data.type_().hint(), Some(state.type_));
                        match state.type_ {
                            TypeHint::Plaintext | TypeHint::Html => {
                                let text = data.try_as_string().unwrap();
                                info!("{text:?}");
                            },
                            TypeHint::UriList => {
                                let uris = data.try_as_uris().unwrap();
                                info!("{uris:#?}");
                            },
                            _ => {
                                unreachable!("Received a type we didn't ask for!");
                            },
                        }
                    } else {
                        info!("Never received");
                    }
                }

                self.last_dnd_fetch = None;
            },
            WindowEvent::DataTransferResult { serial, .. } => {
                info!("{event:?}");

                if let Some(state) = &mut self.last_dnd_fetch
                    && state.serial == serial
                {
                    state.received = true;
                }
            },
            WindowEvent::DragEntered { id } => {
                info!("{event:?}");

                let data_transfer = match event_loop.data_transfer(id) {
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

                let wanted_types = [TypeHint::Html, TypeHint::UriList, TypeHint::Plaintext]
                    .into_iter()
                    .filter(|ty| data_transfer.has_type(ty))
                    .collect::<Vec<_>>();

                info!("Supported types: {:#?}", wanted_types);

                let Some(type_) = wanted_types.first().copied() else {
                    event_loop.reject_drag(id).unwrap();
                    return;
                };

                event_loop.accept_drag_type(id, &type_).unwrap();

                self.last_dnd_fetch = event_loop
                    .fetch_data_transfer(id, &type_)
                    .ok()
                    .map(|serial| FetchState { serial, type_, received: false });
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
