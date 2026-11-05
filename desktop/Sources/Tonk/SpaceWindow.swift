import HarnessCore
import SwiftUI

/// What a space window shows. Opening the same request again brings that
/// window forward instead of making another.
struct SpaceWindowRequest: Codable, Hashable {
  let subject: String
  let url: URL?
}

/// One space view with its own chat. Each window has its own page and chat
/// model, so several spaces, or several views of one space, sit side by side.
struct SpaceWindow: View {
  let request: SpaceWindowRequest
  @ObservedObject var runtime: RuntimeModel
  @ObservedObject var access: AccessModel
  @StateObject private var model: HarnessModel
  @StateObject private var page: SpacePage
  @Environment(\.dismiss) private var dismiss

  init(
    request: SpaceWindowRequest, runtime: RuntimeModel, access: AccessModel, archive: ChatArchive
  ) {
    self.request = request
    self.runtime = runtime
    self.access = access
    _model = StateObject(wrappedValue: HarnessModel(store: archive.handle(claimsChats: true)))
    _page = StateObject(wrappedValue: runtime.makePage())
  }

  var body: some View {
    ContentView(model: model, runtime: runtime, access: access, isSpace: true, spacePage: page)
      .task { await open() }
      .onDisappear {
        model.shutdown()
        page.close()
      }
      // Nothing opened under one account may stay open under the next.
      .onChange(of: runtime.accountEpoch) { dismiss() }
  }

  private func open() async {
    model.runtime = runtime
    model.page = page
    if ActivityFixture.requested {
      ActivityFixture.prepare(model: model, page: page)
      return
    }
    guard let listed = runtime.spaces.first(where: { $0.subject == request.subject }),
      let space = await runtime.spaceForOpening(listed),
      await model.enterSpace(space)
    else {
      dismiss()
      return
    }
    page.open(space, at: request.url)
  }
}

/// The space's page, with its loading and error states.
struct SpaceWorkspace: View {
  @ObservedObject var page: SpacePage
  var viewTitle: String? = nil

  var body: some View {
    ZStack {
      RuntimeView(webView: page.webView)
        .allowsHitTesting(page.space != nil)
        .accessibilityHidden(page.space == nil)
      if let error = page.error {
        ContentUnavailableView {
          Label("Couldn’t open this space", systemImage: "network")
        } description: {
          Text(error)
        } actions: {
          Button("Try again") { page.reload() }
        }.background(.background)
      }
    }
    .background(.background)
    .overlay {
      if page.loading {
        VStack(spacing: 10) {
          ProgressView().controlSize(.small)
          Text("Opening \(viewTitle ?? page.space?.title ?? "space")…")
            .font(.system(size: 13))
          Text("Getting the latest from \(page.space?.title ?? "your space")")
            .font(.system(size: 12)).foregroundStyle(.secondary)
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
          .background(Color(nsColor: .windowBackgroundColor)).allowsHitTesting(false)
      }
    }
  }
}

/// The toolbar spinner while this window's page or the catalog is refreshing.
struct PageActivity: View {
  @ObservedObject var page: SpacePage
  let catalogLoading: Bool

  var body: some View {
    if page.loading || catalogLoading {
      ProgressView()
        .controlSize(.small)
        .frame(width: 16, height: 16)
        .accessibilityLabel("Refreshing space")
        .help("Refreshing…")
    }
  }
}
