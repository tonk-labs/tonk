import AppKit
import SwiftUI
import WebKit

@MainActor
final class PasskeyProbe: NSObject, ObservableObject, WKNavigationDelegate {
  @Published var status = "Loading Tonk…"
  @Published var ready = false
  @Published var pending = false
  let webView: WKWebView

  override init() {
    let configuration = WKWebViewConfiguration()
    // This probe never touches the desktop app's saved account or cookies.
    configuration.websiteDataStore = .nonPersistent()
    webView = WKWebView(frame: .zero, configuration: configuration)
    super.init()
    webView.navigationDelegate = self
    webView.load(URLRequest(url: URL(string: "https://tonk.network")!))
  }

  func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
    ready = webView.url?.host == "tonk.network"
    report(ready ? "Ready to test your existing Tonk passkey." : "Unexpected page.")
  }

  func webView(
    _ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
    decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
  ) {
    if action.targetFrame?.isMainFrame == false {
      decisionHandler(.allow)
      return
    }
    let url = action.request.url
    decisionHandler(
      url?.scheme == "https" && url?.host == "tonk.network"
        && url?.user == nil && url?.password == nil && (url?.port ?? 443) == 443
        ? .allow : .cancel)
  }

  func webView(
    _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!,
    withError error: Error
  ) {
    report("Tonk could not load (code \((error as NSError).code)).")
  }

  // Call WebKit directly from the native button, before any asynchronous work.
  // Only presence/length checks cross into Swift, never credential or PRF bytes.
  func run() {
    guard ready, !pending else { return }
    // WebAuthn requires a focused document even when the action starts in SwiftUI.
    NSApp.activate(ignoringOtherApps: true)
    webView.window?.makeKeyAndOrderFront(nil)
    webView.window?.makeFirstResponder(webView)
    pending = true
    report("Choose your Tonk passkey in the system sheet.")
    webView.callAsyncJavaScript(
      Self.assertion, arguments: [:], in: nil, in: .page
    ) { [weak self] result in
      guard let self else { return }
      self.pending = false
      switch result {
      case .success(let value):
        guard let result = value as? [String: Any] else {
          self.report("Unexpected probe response.")
          return
        }
        if result["ok"] as? Bool == true {
          self.report("Passkey approved. Both 32-byte PRF outputs received. No account was changed.")
        } else {
          self.report("Passkey probe: \(result["reason"] as? String ?? "unknown failure").")
          if let detail = result["detail"] as? String {
            self.status += "\n" + detail
          }
        }
      case .failure(let error):
        self.report("WebKit call failed (code \((error as NSError).code)).")
      }
    }
  }

  func cancel() {
    webView.evaluateJavaScript("window.__tonkPasskeyProbe?.abort()") { _, _ in }
  }

  private func report(_ message: String) {
    status = message
    print(message)
    fflush(stdout)
  }

  static let assertion = """
    if (location.origin !== 'https://tonk.network')
      return {ok: false, reason: 'unexpected origin'};
    if (!window.PublicKeyCredential || !navigator.credentials?.get)
      return {ok: false, reason: 'WebAuthn unavailable'};
    const controller = new AbortController();
    window.__tonkPasskeyProbe = controller;
    let first, second;
    try {
      const credential = await navigator.credentials.get({
        signal: controller.signal,
        publicKey: {
          rpId: 'tonk.network',
          challenge: crypto.getRandomValues(new Uint8Array(32)),
          userVerification: 'required',
          timeout: 60000,
          extensions: {prf: {eval: {
            first: new TextEncoder().encode('tonk/custody/key/v1'),
            second: new TextEncoder().encode('tonk/custody/kek/v1')
          }}}
        }
      });
      const output = credential?.getClientExtensionResults()?.prf?.results;
      if (output?.first) first = new Uint8Array(output.first);
      if (output?.second) second = new Uint8Array(output.second);
      return first?.length === 32 && second?.length === 32
        ? {ok: true} : {ok: false, reason: 'missing PRF outputs'};
    } catch (error) {
      const known = ['NotAllowedError', 'SecurityError', 'NotSupportedError',
                     'AbortError', 'InvalidStateError'];
      // DOMException diagnostics describe WebKit policy failures, not credential output.
      // Display them locally; keep them out of stdout and never return arbitrary errors.
      const detail = error instanceof DOMException ? error.message.slice(0, 400) : '';
      return {ok: false, reason: known.includes(error?.name) ? error.name : 'assertion failed', detail};
    } finally {
      first?.fill(0);
      second?.fill(0);
      if (window.__tonkPasskeyProbe === controller) delete window.__tonkPasskeyProbe;
    }
    """
}

struct ProbeWebView: NSViewRepresentable {
  let webView: WKWebView
  func makeNSView(context: Context) -> WKWebView { webView }
  func updateNSView(_ nsView: WKWebView, context: Context) {}
}

@main
struct PasskeySpike: App {
  @StateObject private var probe = PasskeyProbe()
  var body: some Scene {
    WindowGroup("Tonk Passkey Test") {
      VStack(spacing: 20) {
        Text("Tonk passkey test").font(.title)
        Text("Test the system passkey sheet without signing in or changing your account.")
          .multilineTextAlignment(.center)
        Button("Test passkey") { probe.run() }
          .buttonStyle(.borderedProminent).disabled(!probe.ready || probe.pending)
        if probe.pending { Button("Cancel") { probe.cancel() } }
        Text(probe.status).textSelection(.enabled).multilineTextAlignment(.center)
        // Keep WebKit attached to the presenting window without displaying the Hub.
        ProbeWebView(webView: probe.webView).frame(height: 1).clipped().accessibilityHidden(true)
      }.padding(32).frame(width: 460, height: 280)
    }
  }
}
