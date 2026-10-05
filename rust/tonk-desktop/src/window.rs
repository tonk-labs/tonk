//! The native window and its webview.

use anyhow::Result;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use tokio::runtime::Runtime;
use wry::WebViewBuilder;

/// Defined before any page script runs, in every frame. The page reads
/// it to skip registering a service worker.
const NATIVE_HOST_SCRIPT: &str =
    "globalThis.tonkNativeHost = Object.freeze({ kind: \"desktop\" });";

/// Open the window on `launch_url` and run the event loop until it
/// closes. `runtime` serves the page meanwhile, so it lives as long as
/// the loop.
pub fn run(runtime: Runtime, launch_url: String, origin: String) -> Result<()> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("tonk")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 860.0))
        .build(&event_loop)?;

    let builder = WebViewBuilder::new()
        .with_url(&launch_url)
        .with_initialization_script(NATIVE_HOST_SCRIPT)
        .with_navigation_handler({
            let origin = origin.clone();
            move |url| {
                let inside = stays_inside(&origin, &url);
                if !inside {
                    let _ = webbrowser::open(&url);
                }
                inside
            }
        })
        .with_new_window_req_handler({
            let origin = origin.clone();
            move |url, _features| {
                if !stays_inside(&origin, &url) {
                    let _ = webbrowser::open(&url);
                }
                wry::NewWindowResponse::Deny
            }
        });

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let webview = builder.build(&window)?;
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let webview = {
        use tao::platform::unix::WindowExtUnix as _;
        use wry::WebViewBuilderExtUnix as _;
        let container = window
            .default_vbox()
            .ok_or_else(|| anyhow::anyhow!("the window has no container for the webview"))?;
        builder.build_gtk(container)?
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        // Owned by the loop so they live exactly as long as it does.
        let _ = (&runtime, &webview);
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    })
}

/// Whether a navigation stays in the app. Anything else opens in the
/// system browser instead of replacing the app in its own window.
///
/// Sealed guest frames are `srcdoc` documents (`about:srcdoc`), and the
/// guest runtime loads code from `blob:` and `data:` URLs.
fn stays_inside(origin: &str, url: &str) -> bool {
    url.strip_prefix(origin)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
        || url.starts_with("about:")
        || url.starts_with("blob:")
        || url.starts_with("data:")
}

#[cfg(test)]
mod tests {
    use super::stays_inside;

    #[test]
    fn it_keeps_same_origin_and_frame_urls_inside() {
        let origin = "http://127.0.0.1:4000";
        assert!(stays_inside(origin, "http://127.0.0.1:4000/space/x"));
        assert!(stays_inside(origin, "http://127.0.0.1:4000"));
        assert!(stays_inside(origin, "about:srcdoc"));
        assert!(!stays_inside(origin, "http://127.0.0.1:40001/"));
        assert!(!stays_inside(origin, "https://tonk.network/"));
    }
}
