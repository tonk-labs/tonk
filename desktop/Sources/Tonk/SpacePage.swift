import AppKit
import HarnessCore
import WebKit

/// One space window's page. Each window has its own web view on the account's
/// website data, so it shares the account and worker but loads, fails and is
/// inspected on its own.
@MainActor
final class SpacePage: NSObject, ObservableObject, WKNavigationDelegate, WKUIDelegate {
  @Published private(set) var location: URL?
  @Published private(set) var space: TonkSpace?
  @Published private(set) var loading = false
  @Published private(set) var error: String?
  let webView: WKWebView
  let inspection = RuntimeInspection()

  init(dataStore: WKWebsiteDataStore) {
    let configuration = WKWebViewConfiguration()
    configuration.websiteDataStore = dataStore
    // The native picker can cover/detach this view while still querying its worker.
    configuration.preferences.inactiveSchedulingPolicy = .none
    RuntimeModel.hideFab(in: configuration.userContentController)
    RuntimeModel.identifyShell(in: configuration.userContentController)
    inspection.install(on: configuration.userContentController)
    webView = WKWebView(frame: .zero, configuration: configuration)
    super.init()
    webView.navigationDelegate = self
    webView.uiDelegate = self
    webView.isInspectable = true
  }

  /// Shows `space`, at `url` when that is one of its views.
  func open(_ space: TonkSpace, at url: URL? = nil) {
    loading = true
    self.space = space
    error = nil
    let target = url.flatMap { space.contains($0) ? $0 : nil } ?? space.url
    location = target
    webView.load(URLRequest(url: target))
  }

  /// Reloads, staying on the open view.
  func reload() {
    guard let space else { return }
    open(space, at: webView.url)
  }

  /// Leaves the page blank once its window has closed.
  func close() {
    space = nil
    location = nil
    loading = false
    error = nil
    webView.loadHTMLString("", baseURL: nil)
  }

  /// Whether the page shows `space` right now, for inspection.
  func shows(_ space: TonkSpace) -> Bool {
    guard self.space?.id == space.id, let url = webView.url else { return false }
    return space.contains(url)
  }

  /// Fixture and test pages show static content with no space behind them.
  func showFixture(_ space: TonkSpace, html: String, loading: Bool = false) {
    self.space = space
    self.loading = loading
    location = nil
    error = nil
    if !loading { webView.loadHTMLString(html, baseURL: nil) }
  }

  func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
    inspection.reset()
    loading = true
    error = nil
  }

  func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
    loading = false
    location = webView.url
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

  func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
    loading = false
    error = "This space stopped. Reload to reconnect."
  }

  private func fail(_ failure: Error) {
    if (failure as NSError).code == NSURLErrorCancelled { return }
    loading = false
    error = failure.localizedDescription
  }

  func webView(
    _ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
    decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
  ) {
    decisionHandler(RuntimeModel.policy(for: navigationAction))
  }

  func webView(
    _ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
    for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures
  ) -> WKWebView? {
    RuntimeModel.openExternally(navigationAction)
    return nil
  }
}
