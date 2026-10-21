import HarnessCore
import SwiftUI

struct SpaceShareControl: View {
  let space: TonkSpace
  @ObservedObject var access: AccessModel
  var compact = false
  @State private var copied = false
  @State private var copying = false
  @State private var publishing = false

  /// Copy link and Publish as two glass buttons in one group; Copy link carries the accent.
  var body: some View {
    GlassControls {
      HStack(spacing: 6) {
        Button {
          guard !copying else { return }
          copying = true
          Task {
            copied = await access.copyDefaultLink(in: space)
            copying = false
            try? await Task.sleep(for: .milliseconds(1500))
            copied = false
          }
        } label: {
          Label(copied ? "Copied" : "Copy link", systemImage: copied ? "checkmark" : "link")
            .labelStyle(ShareLabelStyle(compact: compact))
        }
        .nativeControl(prominent: true).disabled(copying)
        .help(copied ? "Copied" : "Copy the space’s default link")
        .accessibilityLabel(copied ? "Copied" : "Copy link")
        Button {
          publishing = true
        } label: {
          Label("Publish", systemImage: "globe").labelStyle(ShareLabelStyle(compact: compact))
        }
        .nativeControl().help("Publish a read-only link").accessibilityLabel("Publish")
      }.fixedSize()
    }
    .sheet(isPresented: $publishing) { PublishSpaceSheet(space: space, access: access) }
  }
}

private struct ShareLabelStyle: LabelStyle {
  let compact: Bool
  @ViewBuilder func makeBody(configuration: Configuration) -> some View {
    if compact {
      // Icon only, still named by its title for VoiceOver.
      Label(configuration).labelStyle(.iconOnly)
    } else {
      HStack(spacing: 6) {
        configuration.icon
        configuration.title
      }
    }
  }
}

struct PublishSpaceSheet: View {
  let space: TonkSpace
  @ObservedObject var access: AccessModel
  @Environment(\.dismiss) private var dismiss
  @State private var changing = false
  @State private var confirmUnpublish = false
  private var publication: AccessLink? {
    access.links(for: space.id).first { $0.kind == .publicRead }
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      Text("Publish \(space.title)").font(.system(size: 15, weight: .semibold))
      Text(
        "Published read-only: anyone with the link can read what it opens. Nobody can change anything through it."
      )
      .font(.system(size: 12)).foregroundStyle(.secondary)
      VStack(spacing: 12) {
        HStack {
          Text("Opens").frame(width: 92, alignment: .leading)
          Text("Whole space")
          Spacer()
        }
        Divider()
        HStack {
          Text("Expires").frame(width: 92, alignment: .leading)
          Text("Never")
          Spacer()
        }
      }.padding(14).background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
      if let publication, let url = access.storedLink(publication) {
        HStack {
          Text(url).font(.system(size: 12, design: .monospaced)).lineLimit(1).truncationMode(
            .middle)
          Spacer(minLength: 8)
          Button("Copy") { access.copyLink(publication) }
        }.padding(12).background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 8))
      }
      if changing {
        HStack(spacing: 8) {
          ProgressView().controlSize(.small)
          Text("Updating public access…").foregroundStyle(.secondary)
        }
      }
      HStack {
        if publication != nil {
          Button("Unpublish…") { confirmUnpublish = true }
        }
        Spacer()
        if publication == nil {
          Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
        } else {
          Button("Done") { dismiss() }.keyboardShortcut(.defaultAction).buttonStyle(
            .borderedProminent)
        }
        if publication == nil {
          Button("Publish and Copy") {
            changing = true
            Task {
              await access.make(.publicRead, in: space)
              changing = false
            }
          }.buttonStyle(.borderedProminent)
        }
      }
    }.font(.system(size: 13)).padding(22).frame(width: 460).disabled(changing)
      .confirmationDialog("Unpublish this space?", isPresented: $confirmUnpublish) {
        Button("Unpublish", role: .destructive) {
          guard let publication else { return }
          changing = true
          Task {
            await access.revoke(publication, space: space)
            changing = false
          }
        }
      } message: {
        Text("The public link will stop giving readers access.")
      }
  }
}

struct SpaceSyncControl: View {
  @ObservedObject var runtime: RuntimeModel
  let space: TonkSpace
  @StateObject private var observation = DesktopSyncState()
  @State private var showing = false
  @State private var hoverTask: Task<Void, Never>?
  var body: some View {
    Button {
      showing.toggle()
    } label: {
      Image(systemName: observation.symbol).font(.system(size: 12, weight: .semibold))
        .frame(width: 20, height: 24)
    }.buttonStyle(.plain).accessibilityLabel(
      "Sync status for \(space.title): \(observation.status)"
    )
    .help(observation.status)
    .onHover { over in
      hoverTask?.cancel()
      if over {
        hoverTask = Task {
          do { try await Task.sleep(for: .milliseconds(350)) } catch { return }
          showing = true
        }
      }
    }
    .onDisappear { hoverTask?.cancel() }
    // Hangs below the circle, as in frame 4; the default edge opens it off the window.
    .popover(isPresented: $showing, arrowEdge: .bottom) {
      SyncStatusView(runtime: runtime, space: space, observation: observation)
    }
    .task(id: space.id) {
      while !Task.isCancelled {
        await observation.check(runtime: runtime, space: space)
        do { try await Task.sleep(for: .seconds(15)) } catch { return }
      }
    }
  }
}
