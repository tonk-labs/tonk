import AppKit
import HarnessCore
import WebKit

@MainActor
extension RuntimeModel {
  /// Whether `email` is an address the access service can enroll. Checked before
  /// the lookup, whose 404 also answers malformed addresses.
  static func isPlausibleEmail(_ email: String) -> Bool {
    guard email.utf8.count <= 254, !email.contains(where: \.isWhitespace) else { return false }
    let parts = email.split(separator: "@", omittingEmptySubsequences: false)
    guard parts.count == 2, !parts[0].isEmpty, !parts[0].contains("/") else { return false }
    let labels = parts[1].split(separator: ".", omittingEmptySubsequences: false)
    return labels.count >= 2
      && labels.allSatisfy { label in
        !label.isEmpty && label.allSatisfy { $0.isLetter || $0.isNumber || $0 == "-" }
      }
  }

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

  /// Where a signup stands once its passkey exists: `none` when no account was
  /// linked, `unenrolled` when the service never received the enrollment.
  static let signUpProgressScript = """
    const account = await api('/api/account');
    if (account.status !== 'registered') return {state: 'none'};
    const customer = await api('/api/customer');
    const states = {Active: 'active', Registered: 'registered', Suspended: 'suspended'};
    return {state: customer.status ? states[customer.status] || 'registered' : 'unenrolled'};
    """

  static let resumeEnrollmentScript = """
    if (typeof window.tonkIdentity?.usePasskey !== 'function')
      throw new Error('Finishing an account here is unavailable. Use the browser option.');
    const root = await api('/api/identity/root');
    if (!root.rootDid || !root.credentialId) throw new Error('This Mac holds no Tonk account.');
    // Assert the passkey that created the account; the worker reuses the account
    // it recorded rather than minting a new one, then enrolls the address.
    await window.tonkIdentity.usePasskey({
      credentialId: root.credentialId, request: {kind: 'enroll', email}
    });
    return {enrolled: true};
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
    guard Self.isPlausibleEmail(email) else {
      accountMessage = "Enter the email address you want to use."
      return
    }
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
    var passkeyCreated = false
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
      passkeyCreated = true
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
      let message =
        (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
        ?? error.localizedDescription
      guard passkeyCreated else {
        confirmingEmail = nil
        accountMessage = "Couldn’t create your Tonk account: \(message)"
        return
      }
      // The worker records the account before enrolling it, so a failure here
      // can leave an account the service never heard of.
      confirmingEmail = email
      attachingAccount = true
      let progress = await signUpProgress()
      attachingAccount = false
      switch progress {
      case "unenrolled":
        enrollmentPending = true
        accountMessage = "Couldn’t finish creating your Tonk account: \(message)"
      case "registered":
        signInPending = false
        nativeSignInPending = false
        await waitForEmailConfirmation()
      default:
        confirmingEmail = nil
        accountMessage = "Couldn’t create your Tonk account: \(message)"
      }
    }
  }

  /// Enrolls the account an interrupted signup created, which sends its
  /// confirmation email, then waits for the address to be confirmed.
  func resumeEnrollment() async {
    guard let email = confirmingEmail, enrollmentPending, !signInPending else { return }
    signInPending = true
    nativeSignInPending = true
    accountMessage = nil
    defer {
      signInPending = false
      nativeSignInPending = false
    }
    do {
      NSApp.activate(ignoringOtherApps: true)
      webView.window?.makeKeyAndOrderFront(nil)
      webView.window?.makeFirstResponder(webView)
      _ = try await accountScript(Self.resumeEnrollmentScript, arguments: ["email": email])
      enrollmentPending = false
    } catch {
      let message =
        (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
        ?? error.localizedDescription
      accountMessage = "Couldn’t finish creating your Tonk account: \(message)"
      return
    }
    signInPending = false
    nativeSignInPending = false
    await waitForEmailConfirmation()
  }

  /// Reads the signup's progress, reloading once if the failed ceremony had
  /// already moved the worker to the new account's profile and retired this page.
  private func signUpProgress() async -> String? {
    var reloaded = false
    for _ in 0..<30 {
      if !webView.isLoading,
        let progress = try? await accountScript(Self.signUpProgressScript)
      {
        return progress["state"] as? String
      }
      if !reloaded {
        reloaded = true
        webView.load(URLRequest(url: RuntimeLocation.home))
      }
      try? await Task.sleep(for: .seconds(1))
    }
    return nil
  }

  /// Leaves the confirmation wait without it. The account works on this Mac, but
  /// does not sync until the address is confirmed.
  func skipEmailConfirmation() {
    confirmingEmail = nil
    enrollmentPending = false
  }

  /// Polls the customer status, which is also what records activation, until the
  /// emailed link has been opened on any device.
  private func waitForEmailConfirmation() async {
    let epoch = accountEpoch
    while confirmingEmail != nil, !enrollmentPending, accountEpoch == epoch, !Task.isCancelled {
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
