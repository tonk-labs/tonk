import AppKit
import HarnessCore
import WebKit

@MainActor
extension RuntimeModel {
  var supportsDirectPasskey: Bool {
    // The signed associated domain is the channel's own remote, so passkeys work only there.
    RuntimeLocation.isChannelHome
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

  static let addPasskeyScript = """
    if (typeof window.tonkIdentity?.addPasskey !== 'function')
      throw new Error('Adding a passkey here is unavailable. Use the browser option.');
    const root = await api('/api/identity/root');
    if (!root.rootDid) throw new Error('Sign in before adding a passkey.');
    // Asserts the passkey that holds the account, then creates the new one for the
    // same relying party the browser uses, so it signs in there too.
    await window.tonkIdentity.addPasskey({request: {
      kind: 'add-passkey', accountDid: root.rootDid, endpoint: runtimeOrigin + '/ucan/'
    }});
    return {added: true};
    """

  /// Creates a passkey in this app's own WebAuthn ceremony. Needs the passkey entitlement;
  /// builds without it hand off to the browser instead.
  func addPasskey() async throws {
    guard supportsDirectPasskey else {
      throw CallbackError("Adding a passkey needs the browser in this build.")
    }
    NSApp.activate(ignoringOtherApps: true)
    let result = try await accountScript(Self.addPasskeyScript)
    guard result["added"] as? Bool == true else {
      throw CallbackError("The passkey was not added.")
    }
  }

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
      // Login can select a retained account branch and retire the old page.
      try await reloadUntilRegistered()
    } catch {
      let message =
        (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
        ?? error.localizedDescription
      accountMessage = "Couldn’t connect Tonk: \(message)"
    }
  }

  /// Reloads the account page and waits for the worker's background attach.
  func reloadUntilRegistered() async throws {
    catalogRecoveryStarted = Date()
    catalogTask?.cancel()
    catalogLoaded = false
    catalogBranch = nil
    spaces = []
    accountEpoch += 1
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
    throw CallbackError(
      "Your passkey was accepted, but account setup is still pending. Reload to check its progress."
    )
  }
}
