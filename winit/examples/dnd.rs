use std::error::Error;

use tracing::{error, info};
use winit::application::ApplicationHandler;
use winit::data_transfer::{TypeHint, TypedData};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
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
    last_dnd_fetch: Option<Box<dyn TypedData>>,
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
        let Some(window) = self.window.as_ref() else {
            return;
        };

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

                if let Some(data) = &mut self.last_dnd_fetch {
                    // This may return an error with `io::ErrorKind::Deadlock` on X11 if
                    // this is called in the event loop thread while the application is
                    // still waiting for data.
                    data.wait_for_data().unwrap();

                    match data.type_().hint() {
                        Some(TypeHint::Plaintext | TypeHint::Html) => {
                            let text = data.try_as_string().unwrap();
                            info!("{text:?}");
                        },
                        Some(TypeHint::UriList) => {
                            let uris = data.try_as_uris().unwrap();
                            info!("{uris:#?}");
                        },
                        _ => {
                            unreachable!("Received a type we didn't ask for!");
                        },
                    }
                }

                self.last_dnd_fetch = None;
            },
            WindowEvent::DragEntered { id, .. } => {
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
                    window.reject_drag(id).unwrap();
                    return;
                };

                window.accept_drag_type(id, &type_).unwrap();

                self.last_dnd_fetch = event_loop.fetch_data_transfer(id, &type_).ok();

                match self.last_dnd_fetch.as_ref().unwrap().wait_for_data() {
                    Err(e) if e.kind() == std::io::ErrorKind::Deadlock => {
                        eprintln!("Immediately waiting for a fetched data transfer may deadlock!");
                    },
                    _ => {},
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
