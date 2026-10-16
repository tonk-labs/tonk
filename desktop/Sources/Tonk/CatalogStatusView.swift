import HarnessCore
import SwiftUI

/// What the detail area shows while the account's spaces load, fail or are empty.
struct CatalogStatusView: View {
  @ObservedObject var runtime: RuntimeModel
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      if let pending = runtime.pendingSpace {
        ProgressView("Opening \(pending.title)…").padding(.horizontal, 24)
      }
      if let error = runtime.catalogError ?? runtime.error {
        VStack(alignment: .leading, spacing: 8) {
          Text("Couldn’t load spaces: \(error)").font(.callout).textSelection(.enabled)
          Button("Try again") {
            if runtime.showingCachedCatalog {
              runtime.load()
            } else {
              Task { await runtime.refreshSpaces() }
            }
          }
        }.padding(.horizontal, 20)
      }
      if runtime.spaces.isEmpty, runtime.catalogError == nil,
        let started = runtime.catalogRecoveryStarted
      {
        TimelineView(.periodic(from: started, by: 1)) { context in
          let delayed = context.date.timeIntervalSince(started) >= 60
          SpaceLoadingView(
            title: delayed ? "Still waiting for your spaces" : "Restoring your spaces…",
            detail: delayed
              ? "Sync is taking longer than expected, or this account has no spaces yet."
              : "You’re signed in. Your spaces will appear here as they arrive.",
            waiting: delayed
          ) {
            if delayed {
              Button("Refresh") { Task { await runtime.refreshSpaces() } }
                .nativeControl().controlSize(.large)
                .disabled(runtime.catalogLoading)
            }
          }
        }
      } else if runtime.spaces.isEmpty
        && (runtime.catalogLoading || (!runtime.catalogLoaded && runtime.catalogError == nil))
      {
        SpaceLoadingView(
          title: "Loading your spaces…", detail: "Connecting to your Tonk workspace.",
          waiting: false
        ) { EmptyView() }
      } else if runtime.spaces.isEmpty && runtime.catalogError != nil {
        ContentUnavailableView(
          "Spaces unavailable", systemImage: "exclamationmark.circle",
          description: Text("Use refresh to try again."))
      } else if runtime.spaces.isEmpty {
        ContentUnavailableView(
          "No spaces yet", systemImage: "square.grid.2x2",
          description: Text("Create a space in Tonk, then refresh to see it here."))
      } else {
        ContentUnavailableView(
          "Choose a space", systemImage: "sidebar.left",
          description: Text("Pick a space in the sidebar to see its people, views and links."))
      }
    }.frame(maxWidth: .infinity, maxHeight: .infinity).background(.background)
  }
}

struct SpaceLoadingView<Action: View>: View {
  let title: String
  let detail: String
  let waiting: Bool
  @ViewBuilder var action: () -> Action

  var body: some View {
    VStack(spacing: 16) {
      ZStack {
        if waiting {
          Image(systemName: "clock").font(.system(size: 28)).foregroundStyle(.secondary)
        } else {
          ProgressView().controlSize(.large)
        }
      }
      .frame(width: 40, height: 40)
      .accessibilityHidden(true)
      VStack(spacing: 10) {
        Text(title).font(.title2.weight(.semibold))
        Text(detail).font(.body).foregroundStyle(.secondary)
          .fixedSize(horizontal: false, vertical: true)
      }
      action()
    }
    .multilineTextAlignment(.center)
    .frame(maxWidth: 360)
    .padding(32)
    .frame(maxWidth: .infinity, maxHeight: .infinity)
  }
}
