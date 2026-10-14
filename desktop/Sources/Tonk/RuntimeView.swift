import AppKit
import HarnessCore
import SwiftUI
import WebKit

@MainActor
final class RuntimeModel: NSObject, ObservableObject, WKNavigationDelegate, WKUIDelegate {
  @Published var loading = true
  @Published var error: String?
  @Published var ready = false
  @Published var signInPending = false
  @Published var nativeSignInPending = false
  @Published var attachingAccount = false
  @Published var accountMessage: String?
  @Published var accountConnected = false
  @Published var accountStatusKnown = false
  @Published var spaces: [TonkSpace] = []
  @Published var showingCachedCatalog = false
  @Published var pendingSpace: TonkSpace?
  var catalogCache: SpaceCatalogCache?
  var cachedAccountRoot: String?
  var liveAccountRoot: String?
  @Published var selectedSpace: TonkSpace?
  @Published var catalogRecoveryStarted: Date?
  @Published var catalogLoading = false
  @Published var catalogLoaded = false
  @Published var catalogError: String?
  var catalogRefreshPending = false
  let catalogObservation = RuntimeCatalogObservation()
  var catalogBranch: String?
  var catalogTask: Task<Void, Never>?
  var localFixtureStarted = false
  var callback: BrowserCallback?
  var mcpBridge: LocalRuntimeBridge?
  var mcpConnectionFile: URL?
  let webView: WKWebView
  let inspection = RuntimeInspection()

  override init() {
    let configuration = WKWebViewConfiguration()
    let arguments = ProcessInfo.processInfo.arguments
    // An explicit WebKit profile lets onboarding checks avoid the user's signed-in account.
    if let index = arguments.firstIndex(of: "--web-data-id"),
      arguments.indices.contains(index + 1), let id = UUID(uuidString: arguments[index + 1])
    {
      configuration.websiteDataStore = WKWebsiteDataStore(forIdentifier: id)
    } else if let id = RuntimeLocation.deployment.webDataIdentifier {
      configuration.websiteDataStore = WKWebsiteDataStore(forIdentifier: id)
    } else {
      configuration.websiteDataStore = .default()
    }
    // The native picker can cover/detach this view while still querying its worker.
    configuration.preferences.inactiveSchedulingPolicy = .none
    // Tonk carries this presentation flag into its sealed guest iframes.
    configuration.userContentController.addUserScript(
      WKUserScript(
        source: "window.__tonkHideFab = true;",
        injectionTime: .atDocumentStart, forMainFrameOnly: true, in: .page))
    // User-selected remotes are restricted by the navigation delegate, not a static bundle list.
    inspection.install(on: configuration.userContentController)
    webView = WKWebView(frame: .zero, configuration: configuration)
    super.init()
    catalogObservation.model = self
    configuration.userContentController.add(catalogObservation, name: "catalogChanged")
    webView.navigationDelegate = self
    webView.uiDelegate = self
    webView.isInspectable = true
  }

  func load() {
    loading = true
    error = nil
    ready = false
    if !accountStatusKnown { accountMessage = nil }
    webView.load(URLRequest(url: selectedSpace?.url ?? RuntimeLocation.home))
  }

  func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
    inspection.reset()
    catalogObservation.generation = UUID().uuidString
    loading = true
    error = nil
  }

  func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
    loading = false
    catalogTask?.cancel()
    catalogTask = Task {
      for _ in 0..<15 {
        guard !Task.isCancelled else { return }
        if let status = try? await probe(), status["health"] as? Bool == true {
          await prepareLocalFixtureIfRequested()
          await refreshSpaces()
          return
        }
        try? await Task.sleep(for: .seconds(1))
      }
      if !Task.isCancelled { catalogError = "The Tonk worker is not ready. Reload to try again." }
    }
  }

  func webView(
    _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!,
    withError error: Error
  ) {
    fail(error)
  }

  func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
    fail(error)
  }

  func applyAccountStatus(_ account: [String: Any]) throws {
    guard let status = account["status"] as? String,
      ["registered", "unregistered", "rootMissing"].contains(status)
    else { throw CallbackError("Invalid account status.") }
    accountConnected = status == "registered"
    accountStatusKnown = true
  }

  func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
    loading = false
    ready = false
    error = "The Tonk runtime stopped. Reload to reconnect."
  }

  private func fail(_ failure: Error) {
    if (failure as NSError).code == NSURLErrorCancelled { return }
    loading = false
    ready = false
    error = failure.localizedDescription
  }

  func webView(
    _ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
    decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
  ) {
    guard let url = navigationAction.request.url else {
      decisionHandler(.cancel)
      return
    }
    // The Tonk renderer owns its sandboxed guest frames; restrict top-level navigation only.
    if navigationAction.targetFrame?.isMainFrame == false {
      decisionHandler(.allow)
      return
    }
    if RuntimeLocation.isEmbedded(url) {
      decisionHandler(.allow)
      return
    }
    if navigationAction.navigationType == .linkActivated && RuntimeLocation.isExternal(url) {
      NSWorkspace.shared.open(url)
    }
    decisionHandler(.cancel)
  }

  func webView(
    _ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
    for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures
  ) -> WKWebView? {
    if let url = navigationAction.request.url, RuntimeLocation.isExternal(url) {
      NSWorkspace.shared.open(url)
    }
    return nil
  }

  /// Read-only probe; there is no JavaScript-to-native privileged bridge.
  func probe() async throws -> [String: Any] {
    let result = try await webView.callAsyncJavaScript(
      """
      const controlled = !!navigator.serviceWorker?.controller;
      let health = false;
      if (controlled) {
          try {
              const response = await fetch('/api/health', {signal: AbortSignal.timeout(4000)});
              const type = response.headers.get('content-type') || '';
              health = response.ok && type.includes('json');
          } catch {}
      }
      const site = document.querySelector('tonk-site');
      const frame = site?.querySelector('iframe') || site?.shadowRoot?.querySelector('iframe');
      const mounted = !!frame && frame.getBoundingClientRect().width > 0;
      return { serviceWorker: 'serviceWorker' in navigator, controlled, mounted,
               health, title: document.title, url: location.origin + location.pathname,
               bodyCharacters: document.body?.innerText.length || 0 };
      """, arguments: [:], in: nil, contentWorld: .page)
    let value = result as? [String: Any] ?? [:]
    ready = value["health"] as? Bool == true && value["mounted"] as? Bool == true
    return value
  }
}

struct RuntimeView: NSViewRepresentable {
  @ObservedObject var model: RuntimeModel
  func makeNSView(context: Context) -> WKWebView { model.webView }
  func updateNSView(_ nsView: WKWebView, context: Context) {}
}

/// Extend only the page's blurred background; embedded frame headers need the safe area.
struct RuntimeToolbarBackground: ViewModifier {
  var enabled: Bool

  @ViewBuilder func body(content: Content) -> some View {
    if #available(macOS 26.0, *), enabled {
      content.backgroundExtensionEffect()
    } else {
      content
    }
  }
}
