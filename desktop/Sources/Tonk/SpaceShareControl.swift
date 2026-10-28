import HarnessCore
import SwiftUI

struct SpaceShareControl: View {
  let space: TonkSpace
  @ObservedObject var access: AccessModel
  var compact = false
  /// Where analytics files this control's actions: the main window or a space window.
  var surface: DesktopAnalytics.Surface = .hub
  @State private var copied = false
  @State private var copying = false
  @State private var publishing = false
  @State private var showingOptions = false

  private var publication: AccessLink? {
    access.links(for: space.id).first { $0.kind == .publicRead }
  }

  /// New link with its sharing options, as one accent glass capsule: the face makes
  /// and copies a fresh link in one click; existing links are copied from the space
  /// page. The chevron explains each way to share.
  var body: some View {
    HStack(spacing: 0) {
      Button(action: makeLink) {
        Label(copied ? "Copied" : "New link", systemImage: copied ? "checkmark" : "link.badge.plus")
          .labelStyle(ShareLabelStyle(compact: compact))
          .font(.system(size: 13, weight: .medium))
          .padding(.leading, compact ? 11 : 13).padding(.trailing, compact ? 9 : 10)
          .frame(height: 32).contentShape(Rectangle())
      }
      .buttonStyle(.plain)
      .help(copied ? "Copied" : "Make a new link to this space and copy it")
      .accessibilityLabel(copied ? "Copied" : "New link")
      .disabled(copying)
      Rectangle().fill(.white.opacity(0.4)).frame(width: 1, height: 16)
        .accessibilityHidden(true)
      Button {
        showingOptions.toggle()
      } label: {
        Image(systemName: "chevron.down").font(.system(size: 10, weight: .bold))
          .frame(width: 26, height: 32).contentShape(Rectangle())
      }
      .buttonStyle(.plain)
      .help("Sharing options")
      .accessibilityLabel("Sharing options")
      .popover(isPresented: $showingOptions, arrowEdge: .bottom) {
        VStack(alignment: .leading, spacing: 2) {
          ShareOption(
            title: "New link", detail: "Makes and copies a link anyone can use to view and edit",
            symbol: "link.badge.plus"
          ) {
            showingOptions = false
            makeLink()
          }
          Divider().padding(.horizontal, 8).padding(.vertical, 2)
          if let publication {
            ShareOption(
              title: "Copy public link", detail: "Anyone with it can view, read-only",
              symbol: "globe", enabled: access.storedLink(publication) != nil
            ) {
              showingOptions = false
              access.copyLink(publication, surface: surface)
            }
            ShareOption(
              title: "Publishing settings…", detail: "Copy, review or unpublish",
              symbol: "slider.horizontal.3"
            ) {
              showingOptions = false
              publishing = true
            }
          } else {
            ShareOption(
              title: "Publish…", detail: "Make a read-only link anyone can open",
              symbol: "globe"
            ) {
              showingOptions = false
              publishing = true
            }
          }
        }.padding(6).frame(width: 270)
          // The capsule's white label must not carry into the popover.
          .foregroundStyle(Color.primary)
      }
    }
    .foregroundStyle(.white).fixedSize().accentCapsule()
    .sheet(isPresented: $publishing) {
      PublishSpaceSheet(space: space, access: access, surface: surface)
    }
  }

  private func makeLink() {
    guard !copying else { return }
    copying = true
    Task {
      copied = await access.make(.person, in: space, surface: surface)
      copying = false
      try? await Task.sleep(for: .milliseconds(1500))
      copied = false
    }
  }
}

/// One sharing choice: what it does, and who it lets in.
private struct ShareOption: View {
  let title: String
  let detail: String
  let symbol: String
  var enabled = true
  var action: () -> Void
  @State private var hovering = false

  var body: some View {
    Button(action: action) {
      HStack(alignment: .top, spacing: 10) {
        Image(systemName: symbol).font(.system(size: 14)).frame(width: 18).padding(.top, 1)
          .foregroundStyle(hovering && enabled ? Color.white : Color.accentColor)
        VStack(alignment: .leading, spacing: 2) {
          Text(title).font(.system(size: 13))
          Text(detail).font(.system(size: 11))
            .foregroundStyle(
              hovering && enabled ? AnyShapeStyle(.white.opacity(0.85)) : AnyShapeStyle(.secondary))
        }
        Spacer(minLength: 0)
      }
      .foregroundStyle(hovering && enabled ? Color.white : Color.primary)
      .padding(.horizontal, 8).padding(.vertical, 6)
      .background(
        hovering && enabled ? Color.accentColor : .clear, in: RoundedRectangle(cornerRadius: 6)
      )
      .contentShape(RoundedRectangle(cornerRadius: 6))
    }
    .buttonStyle(.plain).disabled(!enabled).opacity(enabled ? 1 : 0.5)
    .onHover { hovering = $0 }
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
  var surface: DesktopAnalytics.Surface = .hub
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
          Button("Copy") { access.copyLink(publication, surface: surface) }
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
              await access.make(.publicRead, in: space, surface: surface)
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
            await access.revoke(publication, space: space, surface: surface)
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

extension View {
  /// The accent-tinted glass capsule behind the share control; a flat accent fill when
  /// glass is unavailable or transparency is reduced.
  fileprivate func accentCapsule() -> some View { modifier(AccentCapsule()) }
}

private struct AccentCapsule: ViewModifier {
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

  @ViewBuilder func body(content: Content) -> some View {
    if #available(macOS 26.0, *), !reduceTransparency {
      content.glassEffect(.regular.tint(.accentColor).interactive(), in: Capsule())
    } else {
      content.background(Color.accentColor, in: Capsule())
    }
  }
}
