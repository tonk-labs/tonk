//! The native window and its webview.

use anyhow::Result;
use std::sync::Mutex;

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use tokio::runtime::Runtime;
use wry::WebViewBuilder;

/// Runs before any page script, in every frame: marks the page as
/// natively hosted and carries live queries over WebSockets.
const NATIVE_HOST_SCRIPT: &str = include_str!("native_host.js");

/// The native-host script for a page served from `origin`.
pub fn native_host_script(origin: &str) -> String {
    NATIVE_HOST_SCRIPT.replace("__TONK_ORIGIN__", origin)
}

/// A location for the worker's navigator to load: in the window, or in
/// the system browser.
#[derive(Debug, PartialEq, Eq)]
pub enum Destination {
    /// An absolute address on the page's origin.
    Window(String),
    /// Anywhere else, such as a deployment approving a sign-in.
    Browser(String),
}

/// Where the worker's `href` should load for a page served from `origin`.
/// A path is on the page's own origin.
pub fn destination(origin: &str, href: &str) -> Destination {
    if href.starts_with('/') && !href.starts_with("//") {
        Destination::Window(format!("{origin}{href}"))
    } else if stays_inside(origin, href) {
        Destination::Window(href.to_owned())
    } else {
        Destination::Browser(href.to_owned())
    }
}

/// What the event loop is woken for besides window events.
enum Wake {
    /// Load `url` in the webview, replacing the history entry or not.
    Load { url: String, replace: bool },
    /// A menu item was chosen.
    #[cfg(target_os = "macos")]
    Menu(muda::MenuId),
}

/// Open the window on `launch_url` and run the event loop until it
/// closes. `runtime` serves the page meanwhile, so it lives as long as
/// the loop.
pub fn run(runtime: Runtime, launch_url: String, origin: String) -> Result<()> {
    let event_loop = EventLoopBuilder::<Wake>::with_user_event().build();
    // The worker navigates from its own threads; the webview can only be
    // touched from the loop, so a load is sent there as an event.
    let proxy = Mutex::new(event_loop.create_proxy());
    tonk_worker::native::set_navigator({
        let origin = origin.clone();
        move |href, replace| match destination(&origin, href) {
            Destination::Window(url) => {
                if let Ok(proxy) = proxy.lock() {
                    let _ = proxy.send_event(Wake::Load { url, replace });
                }
            }
            Destination::Browser(url) => {
                let _ = webbrowser::open(&url);
            }
        }
    });
    #[cfg(target_os = "macos")]
    {
        let proxy = Mutex::new(event_loop.create_proxy());
        muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
            if let Ok(proxy) = proxy.lock() {
                let _ = proxy.send_event(Wake::Menu(event.id));
            }
        }));
    }
    let window = WindowBuilder::new()
        .with_title("Tonk")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 860.0))
        .build(&event_loop)?;

    let builder = WebViewBuilder::new()
        .with_url(&launch_url)
        .with_devtools(true)
        .with_initialization_script(native_host_script(&origin))
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

    // Installed once the app has started, as the menu bar requires.
    #[cfg(target_os = "macos")]
    let mut menu: Option<crate::menu::MenuBar> = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        // Owned by the loop so they live exactly as long as it does.
        let _ = &runtime;
        match event {
            #[cfg(target_os = "macos")]
            Event::NewEvents(tao::event::StartCause::Init) => match crate::menu::install() {
                Ok(installed) => menu = Some(installed),
                Err(error) => eprintln!("the menu bar could not be installed: {error}"),
            },
            #[cfg(target_os = "macos")]
            Event::UserEvent(Wake::Menu(id)) => {
                match menu.as_ref().and_then(|menu| menu.command(&id)) {
                    Some(crate::menu::Command::Reload) => {
                        let _ = webview.reload();
                    }
                    Some(crate::menu::Command::Inspect) => webview.open_devtools(),
                    None => {}
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            Event::UserEvent(Wake::Load { url, replace }) => {
                let method = if replace { "replace" } else { "assign" };
                let url = serde_json::to_string(&url).unwrap_or_default();
                let _ = webview.evaluate_script(&format!("location.{method}({url})"));
            }
            _ => {}
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
    use super::{Destination, destination, stays_inside};

    #[test]
    fn it_loads_the_pages_own_locations_in_the_window_and_others_in_the_browser() {
        let origin = "http://127.0.0.1:4000";
        assert_eq!(
            destination(origin, "/"),
            Destination::Window("http://127.0.0.1:4000/".into())
        );
        assert_eq!(
            destination(origin, "/space/x?y=1"),
            Destination::Window("http://127.0.0.1:4000/space/x?y=1".into())
        );
        assert_eq!(
            destination(origin, "http://127.0.0.1:4000/space/x"),
            Destination::Window("http://127.0.0.1:4000/space/x".into())
        );
        assert_eq!(
            destination(origin, "https://tonk.network/settings/link?a=1"),
            Destination::Browser("https://tonk.network/settings/link?a=1".into())
        );
        assert_eq!(
            destination(origin, "//evil.example/"),
            Destination::Browser("//evil.example/".into())
        );
    }

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
