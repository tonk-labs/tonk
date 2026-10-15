import AppKit
import HarnessCore
import WebKit

@MainActor
extension RuntimeModel {
  var supportsDirectPasskey: Bool {
    RuntimeLocation.deployment == .production
      && Bundle.main.object(forInfoDictionaryKey: "TonkDirectPasskeyEnabled") as? Bool == true
  }

  static let passkeyPreflightScript = """
    const root = await api('/api/identity/root');
    const account = await api('/api/account');
    if (!root.deviceDid) throw new Error('The runtime did not return a device identity.');
    if (account.status === 'registered') return {connected: true};
    // The browser grant path enforces expectedAccount. Do not silently replace
    // that constraint with custody login's account-profile selection.
    if (root.rootDid) return {browserRequired: true};
    if (typeof window.tonkIdentity?.usePasskey !== 'function')
      throw new Error('This Tonk runtime does not support direct passkey sign-in. Use the browser option.');
    return {ready: true};
    """

  static let passkeyLoginScript = """
    if (typeof window.tonkIdentity?.usePasskey !== 'function')
      throw new Error('Passkey sign-in is unavailable. Use the browser option.');
    // Start synchronously in this WebKit invocation. The page and worker own
    // all PRF material; Swift receives only the operation's completion.
    const result = await window.tonkIdentity.usePasskey({request: {
      kind: 'login', deviceName: 'Tonk',
      endpoint: runtimeOrigin + '/ucan/', provider: runtimeOrigin + '/ucan/'
    }});
    return {accepted: true, reload: result?.reload === true};
    """

  func signInWithPasskey() async {
    guard supportsDirectPasskey, !signInPending else { return }
    signInPending = true
    nativeSignInPending = true
    accountMessage = nil
    defer {
      signInPending = false
      nativeSignInPending = false
      attachingAccount = false
    }
    do {
      let preflight = try await accountScript(Self.passkeyPreflightScript)
      if preflight["connected"] as? Bool == true {
        accountConnected = true
        return
      }
      if preflight["browserRequired"] as? Bool == true {
        accountMessage = "Use Sign in through browser to reconnect this account."
        return
      }
      guard preflight["ready"] as? Bool == true else {
        throw CallbackError("The Tonk runtime is not ready to sign in.")
      }
      NSApp.activate(ignoringOtherApps: true)
      webView.window?.makeKeyAndOrderFront(nil)
      webView.window?.makeFirstResponder(webView)
      let result = try await accountScript(Self.passkeyLoginScript)
      guard result["accepted"] as? Bool == true else {
        throw CallbackError("The passkey operation did not complete.")
      }
      attachingAccount = true
      catalogRecoveryStarted = Date()
      // Login can select a retained account branch and retire the old page.
      // Reload before querying, then wait for the worker's background attach.
      catalogTask?.cancel()
      catalogLoaded = false
      catalogBranch = nil
      spaces = []
      selectedSpace = nil
      webView.load(URLRequest(url: RuntimeLocation.home))
      for _ in 0..<60 {
        try await Task.sleep(for: .seconds(1))
        if webView.isLoading { continue }
        let account = try await accountScript("return await api('/api/account');")
        if account["status"] as? String == "registered" {
          accountConnected = true
          accountMessage = nil
          await refreshSpaces()
          return
        }
      }
      throw CallbackError("Your passkey was accepted, but account setup is still pending. Reload to check its progress.")
    } catch {
      let message = (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
        ?? error.localizedDescription
      accountMessage = "Couldn’t connect Tonk: \(message)"
    }
  }
}
