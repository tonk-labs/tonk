import AppKit
import SwiftUI

struct TonkWelcomeView: View {
  @ObservedObject var runtime: RuntimeModel
  @State private var creatingAccount = false
  @State private var email = ""
  @State private var displayName = ""

  var body: some View {
    VStack(spacing: 24) {
      Spacer()
      Image(nsImage: NSWorkspace.shared.icon(forFile: Bundle.main.bundlePath))
        .resizable()
        .scaledToFit()
        .frame(width: 80, height: 80)
        .accessibilityHidden(true)
      Text("Welcome to Tonk").font(.largeTitle.weight(.semibold))
      VStack(spacing: 14) {
        if let address = runtime.confirmingEmail, !runtime.signInPending {
          confirmation(address)
        } else if runtime.signInPending {
          if let progress = runtime.browserSignInProgress {
            BrowserSignInProgressView(progress: progress)
          } else {
            ProgressView(
              runtime.attachingAccount
                ? "Connecting your account…"
                : creatingAccount && !runtime.enrollmentPending
                  ? "Create your Tonk passkey…" : "Approve your Tonk passkey…")
          }
          if !runtime.nativeSignInPending {
            Button("Cancel") { runtime.cancelSignIn() }.disabled(runtime.attachingAccount)
          }
        } else if runtime.loading
          || !runtime.catalogLoaded && runtime.catalogError == nil && runtime.error == nil
        {
          ProgressView("Connecting to Tonk…")
        } else if creatingAccount {
          accountForm
        } else {
          HStack(spacing: 12) {
            Button("Sign in to Tonk", systemImage: "person.crop.circle.badge.checkmark") {
              Task { await runtime.signIn() }
            }.nativeControl(prominent: true).controlSize(.large)
              .disabled(runtime.loading)
            if runtime.supportsDirectPasskey {
              Button("Create account") {
                runtime.accountMessage = nil
                creatingAccount = true
              }
              .nativeControl().controlSize(.large)
            }
          }
          if runtime.supportsDirectPasskey {
            Button("Sign in through browser") { Task { await runtime.signInInBrowser() } }
              .buttonStyle(.link)
          } else {
            Text(
              "Use your existing Tonk passkey in your browser.\nYou’ll return here when sign-in is complete."
            )
            .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
          }
        }
        if let message = runtime.accountMessage ?? runtime.error
          ?? (runtime.signInPending ? nil : runtime.catalogError)
        {
          Text(message).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
          if !runtime.signInPending && !creatingAccount && runtime.confirmingEmail == nil {
            Button("Try again") { runtime.load() }
          }
        }
      }.frame(maxWidth: 520)
      Spacer()
    }.padding(32).frame(maxWidth: .infinity, maxHeight: .infinity).background(.background)
  }

  private var accountForm: some View {
    VStack(spacing: 14) {
      Form {
        TextField("Email", text: $email, prompt: Text(verbatim: "you@example.com"))
          .textContentType(.emailAddress)
        TextField("Display name", text: $displayName, prompt: Text("How others see you"))
          .textContentType(.name)
      }
      .formStyle(.columns)
      .frame(maxWidth: 360)
      .onSubmit(createAccount)
      Text("Tonk creates a passkey on this Mac. You won’t need a password.")
        .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
      HStack(spacing: 12) {
        Button("Back") {
          runtime.accountMessage = nil
          creatingAccount = false
        }
        .nativeControl().controlSize(.large)
        Button("Create account", systemImage: "person.crop.circle.badge.plus", action: createAccount)
          .nativeControl(prominent: true).controlSize(.large)
          .keyboardShortcut(.defaultAction)
          .disabled(!canCreateAccount)
      }
    }
  }

  private func confirmation(_ address: String) -> some View {
    VStack(spacing: 14) {
      Image(systemName: "envelope.badge").font(.system(size: 32)).foregroundStyle(.secondary)
        .accessibilityHidden(true)
      if runtime.enrollmentPending {
        Text("Finish creating your account").font(.title2.weight(.semibold))
        Text("Your passkey is ready, but Tonk hasn’t sent the confirmation email to \(address) yet.")
          .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
        Button("Try again", systemImage: "arrow.clockwise") {
          Task { await runtime.resumeEnrollment() }
        }
        .nativeControl(prominent: true).controlSize(.large)
      } else {
        Text("Confirm your email").font(.title2.weight(.semibold))
        Text("Open the link we sent to \(address).")
          .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
        ProgressView().controlSize(.small)
      }
    }
  }

  private var canCreateAccount: Bool {
    RuntimeModel.isPlausibleEmail(email.trimmingCharacters(in: .whitespacesAndNewlines))
      && !displayName.trimmingCharacters(in: .whitespaces).isEmpty
  }

  private func createAccount() {
    guard canCreateAccount else { return }
    Task { await runtime.signUpWithPasskey(email: email, displayName: displayName) }
  }
}
