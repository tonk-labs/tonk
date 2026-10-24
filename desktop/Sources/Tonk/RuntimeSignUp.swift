import AppKit
import HarnessCore
import WebKit

@MainActor
extension RuntimeModel {
  /// Asks the access service whether `email` already has an account before any
  /// passkey is created. Creating one for a taken address would leave an orphan
  /// passkey in the person's authenticator and fail at the end.
  static let signUpLookupScript = """
    const address = email.trim();
    const at = address.lastIndexOf('@');
    const local = address.slice(0, at).toLowerCase();
    const domain = address.slice(at + 1).toLowerCase();
    if (at < 1 || !domain || local.includes('/') || domain.includes('/'))
      return {state: 'invalid'};
    const response = await fetch(
      '/customer/' + encodeURIComponent(domain) + '/' + encodeURIComponent(local) + '/did.json',
      {signal: AbortSignal.timeout(20000)});
    const states = {200: 'active', 202: 'pending', 404: 'unregistered', 410: 'suspended'};
    return {state: states[response.status] || 'unavailable'};
    """

  static let signUpScript = """
    if (typeof window.tonkIdentity?.createPasskey !== 'function')
      throw new Error('Creating an account here is unavailable. Use the browser option.');
    // The worker creates the account secret, seals it under the new passkey and
    // enrolls the address. Swift receives only the operation's completion.
    const result = await window.tonkIdentity.createPasskey({
      name: email, displayName: email,
      request: {
        kind: 'create-account', email, displayName, deviceName: 'Tonk',
        remote: runtimeOrigin + '/ucan/', provider: runtimeOrigin + '/ucan/',
        createdOn: 'Tonk on macOS'
      }
    });
    const answer = result?.ok ?? result;
    return {created: true, reload: answer?.reload === true};
    """

  static let signUpConfirmationScript = """
    const customer = await api('/api/customer');
    return {active: customer.status === 'Active'};
    """

  /// Creates a Tonk account held by a new passkey on this Mac, then waits for the
  /// address to be confirmed from the emailed link.
  func signUpWithPasskey(email: String, displayName: String) async {
    guard supportsDirectPasskey, !signInPending else { return }
    let email = email.trimmingCharacters(in: .whitespacesAndNewlines)
    let displayName = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !displayName.isEmpty else {
      accountMessage = "Enter a display name to continue."
      return
    }
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
        accountMessage = "This Mac already holds a Tonk account. Sign in to reconnect it."
        return
      }
      guard preflight["ready"] as? Bool == true else {
        throw CallbackError("The Tonk runtime is not ready to create an account.")
      }
      let lookup = try await accountScript(Self.signUpLookupScript, arguments: ["email": email])
      switch lookup["state"] as? String {
      case "unregistered": break
      case "active", "pending":
        accountMessage = "\(email) already has a Tonk account. Sign in instead."
        return
      case "invalid":
        accountMessage = "Enter the email address you want to use."
        return
      case "suspended":
        accountMessage = "\(email) can’t be used for a new Tonk account."
        return
      default:
        throw CallbackError("Tonk couldn’t check that address. Try again in a moment.")
      }
      NSApp.activate(ignoringOtherApps: true)
      webView.window?.makeKeyAndOrderFront(nil)
      webView.window?.makeFirstResponder(webView)
      let result = try await accountScript(
        Self.signUpScript, arguments: ["email": email, "displayName": displayName])
      guard result["created"] as? Bool == true else {
        throw CallbackError("The passkey operation did not complete.")
      }
      // Hold the account picker back until the address is confirmed: the
      // account cannot sync before then.
      confirmingEmail = email
      attachingAccount = true
      if result["reload"] as? Bool == true {
        // Creation selected a new account profile and retired this page.
        try await reloadUntilRegistered()
      } else {
        accountConnected = true
      }
      attachingAccount = false
      signInPending = false
      nativeSignInPending = false
      await waitForEmailConfirmation()
    } catch {
      confirmingEmail = nil
      let message =
        (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
        ?? error.localizedDescription
      accountMessage = "Couldn’t create your Tonk account: \(message)"
    }
  }

  /// Leaves the confirmation wait without it. The account works on this Mac, but
  /// does not sync until the address is confirmed.
  func skipEmailConfirmation() {
    confirmingEmail = nil
  }

  /// Polls the customer status, which is also what records activation, until the
  /// emailed link has been opened on any device.
  private func waitForEmailConfirmation() async {
    let epoch = accountEpoch
    while confirmingEmail != nil, accountEpoch == epoch, !Task.isCancelled {
      if let status = try? await accountScript(Self.signUpConfirmationScript),
        status["active"] as? Bool == true
      {
        confirmingEmail = nil
        await refreshSpaces()
        return
      }
      try? await Task.sleep(for: .seconds(4))
    }
  }
}
