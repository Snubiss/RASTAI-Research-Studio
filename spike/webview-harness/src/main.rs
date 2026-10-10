//! Opens one URL in the same webview Tauri uses (WebKitGTK on Linux), through Tauri's own crates:
//! `tao` for the window and `wry` for the webview. Experiment 06 uses it to measure the canvas
//! exactly where the Rastai desktop app will draw it.
//!
//! Usage: webview-harness <url> [width] [height]

use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use wry::WebViewBuilder;

const WINDOW_TITLE: &str = "Rastai canvas spike (WebKitGTK)";
const DEFAULT_WIDTH: f64 = 1600.0;
const DEFAULT_HEIGHT: f64 = 1000.0;

struct Arguments {
    url: String,
    width: f64,
    height: f64,
}

fn read_arguments() -> Arguments {
    let mut arguments = std::env::args().skip(1);
    let url = arguments.next().unwrap_or_else(|| {
        eprintln!("usage: webview-harness <url> [width] [height]");
        std::process::exit(2);
    });
    let width = arguments.next().and_then(|value| value.parse().ok()).unwrap_or(DEFAULT_WIDTH);
    let height = arguments.next().and_then(|value| value.parse().ok()).unwrap_or(DEFAULT_HEIGHT);
    Arguments { url, width, height }
}

fn main() -> wry::Result<()> {
    let arguments = read_arguments();
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title(WINDOW_TITLE)
        .with_inner_size(LogicalSize::new(arguments.width, arguments.height))
        .build(&event_loop)
        .expect("could not create the window");

    // On Linux the webview lives inside the window's GTK container, exactly as in a Tauri app.
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let container = window.default_vbox().expect("the window has no GTK container");
        WebViewBuilder::new().with_url(&arguments.url).build_gtk(container)?
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            *control_flow = ControlFlow::Exit;
        }
    });
}
