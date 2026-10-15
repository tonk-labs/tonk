import AppKit
import SwiftUI

struct TonkWelcomeView: View {
  @ObservedObject var runtime: RuntimeModel
  var body: some View {
    VStack(spacing: 24) {
      Spacer()
      Image(nsImage: NSWorkspace.shared.icon(forFile: Bundle.main.bundlePath))
        .resizable()
        .scaledToFit()
        .frame(width: 80, height: 80)
        .accessibilityHidden(true)
      VStack(spacing: 12) {
        Text("Welcome to Tonk").font(.largeTitle.weight(.semibold))
        Text("Your spaces, with an agent to help you build.")
          .font(.title3).foregroundStyle(.secondary)
      }
      VStack(spacing: 14) {
        if runtime.signInPending {
          ProgressView(
            runtime.attachingAccount
              ? "Connecting your account…"
              : (runtime.nativeSignInPending ? "Approve your Tonk passkey…" : "Finish signing in in your browser"))
          if !runtime.nativeSignInPending {
            Button("Cancel") { runtime.cancelSignIn() }.disabled(runtime.attachingAccount)
          }
        } else if runtime.loading
          || !runtime.catalogLoaded && runtime.catalogError == nil && runtime.error == nil
        {
          ProgressView("Connecting to Tonk…")
        } else {
          HStack(spacing: 12) {
            Button("Sign in to Tonk", systemImage: "person.crop.circle.badge.checkmark") {
              Task { await runtime.signIn() }
            }.nativeControl(prominent: true).controlSize(.large)
              .disabled(runtime.loading)
            if runtime.supportsDirectPasskey {
              Button("Sign in through browser") { Task { await runtime.signInInBrowser() } }
                .nativeControl().controlSize(.large)
            }
          }
          if !runtime.supportsDirectPasskey {
            Text("Use your existing Tonk passkey in your browser.\nYou’ll return here when sign-in is complete.")
              .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
          }
        }
        if let message = runtime.accountMessage ?? runtime.error ?? runtime.catalogError {
          Text(message).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
          if !runtime.signInPending { Button("Try again") { runtime.load() } }
        }
      }.frame(maxWidth: 520)
      Spacer()
    }.padding(32).frame(maxWidth: .infinity, maxHeight: .infinity).background(.background)
  }
}
