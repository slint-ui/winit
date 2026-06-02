use std::error::Error;

use tracing::{error, info, warn};
use winit::application::ApplicationHandler;
use winit::data_transfer::{DataTransferId, DataTransferSendBuilder, TypeHint, TypedData};
use winit::event::{MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, DndActions, EventLoop};
use winit::icon::{Icon, RgbaIcon};
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
#[derive(Debug)]
struct Application {
    window: Option<Box<dyn Window>>,
    last_dnd_fetch: Option<Box<dyn TypedData>>,
    last_drag_start: Option<DataTransferId>,
    drag_icon: Icon,
}

const DRAG_IMAGE: &[u8] = include_bytes!("data/icon.png");

impl Application {
    fn new() -> Self {
        let drag_icon = load_icon(DRAG_IMAGE);

        Self { window: None, last_dnd_fetch: None, last_drag_start: None, drag_icon }
    }
}

fn load_icon(bytes: &[u8]) -> Icon {
    let (icon_rgba, icon_width, icon_height) = {
        let image = image::load_from_memory(bytes).unwrap().into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };
    RgbaIcon::new(icon_rgba, icon_width, icon_height).expect("Failed to open icon").into()
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
        window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::PointerButton { button, state, .. } => {
                let Some(button) = button.mouse_button() else {
                    return;
                };

                if button == MouseButton::Left && state.is_pressed() {
                    if let Some(last_drag) = self.last_drag_start.take() {
                        let _ = event_loop.cancel_drag(last_drag);
                    }

                    let result = event_loop.start_drag(
                        window_id,
                        DataTransferSendBuilder::new(())
                            .with_type(TypeHint::Plaintext, |()| "Winit example".to_string().into())
                            .with_type(TypeHint::Html, |()| {
                                format!("<strong>Winit</strong> example").into()
                            })
                            .with_type(TypeHint::Image { extension_hint: Some("png") }, |()| {
                                DRAG_IMAGE.to_vec().into()
                            })
                            .build(),
                        &DndActions::new_copy(),
                        Some(self.drag_icon.clone()),
                    );

                    self.last_drag_start = dbg!(result).ok();
                }
            },
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

                info!("Types: {:#?}", data_transfer.available_types());

                let valid_type = [TypeHint::Html, TypeHint::UriList, TypeHint::Plaintext]
                    .into_iter()
                    .find(|ty| data_transfer.has_type(ty));

                let Some(type_) = valid_type else {
                    event_loop.set_valid_actions(id, &DndActions::none()).unwrap();
                    return;
                };

                event_loop.set_valid_actions(id, &DndActions::all()).unwrap();

                self.last_dnd_fetch = event_loop.fetch_data_transfer(id, &type_).ok();

                match self.last_dnd_fetch.as_mut().unwrap().try_as_string() {
                    Err(e) if e.kind() == std::io::ErrorKind::Deadlock => {
                        warn!(
                            "Immediately waiting for a fetched data transfer may deadlock on some \
                             platforms!"
                        );
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
