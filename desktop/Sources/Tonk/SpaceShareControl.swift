import HarnessCore
import SwiftUI

struct SpaceShareControl: View {
  let space: TonkSpace
  @ObservedObject var access: AccessModel
  var compact = false
  @State private var copied = false
  @State private var copying = false
  @State private var publishing = false

  var body: some View {
    HStack(spacing: 0) {
      Button {
        copying = true
        Task {
          copied = await access.copyDefaultLink(in: space)
          copying = false
          try? await Task.sleep(for: .milliseconds(1500))
          copied = false
        }
      } label: {
        HStack(spacing: 6) {
          DesktopGlyph(name: copied ? "imgIconCheck" : "imgIconLink1", size: 14)
          if !compact { Text(copied ? "Copied" : "Copy Link") }
        }
        .padding(.leading, 11).padding(.trailing, 8).frame(height: 28)
      }.buttonStyle(.plain).disabled(copying)
        .accessibilityLabel(copied ? "Copied" : "Copy Link")
      Menu {
        Button("Publish…") { publishing = true }
      } label: {
        Label {
          Text("Sharing options")
        } icon: {
          DesktopGlyph(name: "r2IconChevronDown", size: 12).foregroundStyle(.white)
        }.labelStyle(.iconOnly).frame(width: 26, height: 28)
      }.menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().frame(width: 26, height: 28)
        .tint(.white)
        .accessibilityLabel("Sharing options")
    }
    .font(.system(size: 13, weight: .medium)).foregroundStyle(.white)
    .background(DesktopStyle.accent, in: Capsule())
    .sheet(isPresented: $publishing) { PublishSpaceSheet(space: space, access: access) }
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
        "Anyone with the public link can read this space. Nobody can change it through that link."
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
      if changing { ProgressView("Updating public access…").controlSize(.small) }
      HStack {
        if publication != nil {
          Button("Unpublish…") { confirmUnpublish = true }
        }
        Spacer()
        Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
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
  @State private var held = false
  @State private var hoverTask: Task<Void, Never>?
  var body: some View {
    Button {
      if !held { showing.toggle() }
      held = false
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
    .simultaneousGesture(
      LongPressGesture(minimumDuration: 0.5).onEnded { _ in
        held = true
        Task { await observation.toggle(runtime: runtime, space: space) }
      }
    )
    .popover(isPresented: $showing) {
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
