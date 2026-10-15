import AppKit
import QuartzCore
import SwiftUI

/// Keep both panes mounted while AppKit animates and resizes the split view.
struct AnimatedSidebar<Sidebar: View, Detail: View>: NSViewControllerRepresentable {
  var isVisible: Bool
  var reduceMotion: Bool
  var maximumWidth: CGFloat = 560
  var sidebar: Sidebar
  var detail: Detail
  var onVisibilitySettled: (Bool) -> Void = { _ in }

  func makeNSViewController(context: Context) -> SidebarSplitController<Sidebar, Detail> {
    let controller = SidebarSplitController(sidebar: sidebar, detail: detail, isVisible: isVisible)
    controller.onVisibilitySettled = onVisibilitySettled
    return controller
  }

  func updateNSViewController(
    _ controller: SidebarSplitController<Sidebar, Detail>, context: Context
  ) {
    controller.onVisibilitySettled = onVisibilitySettled
    controller.setMaximumSidebarWidth(maximumWidth)
    controller.sidebarHost.rootView = sidebar
    controller.detailHost.rootView = detail
    controller.setSidebarVisible(isVisible, animated: !reduceMotion)
  }
}

final class SidebarSplitController<Sidebar: View, Detail: View>: NSSplitViewController {
  let sidebarHost: NSHostingController<Sidebar>
  let detailHost: NSHostingController<Detail>
  private let sidebarItem: NSSplitViewItem
  private var requestedVisible: Bool
  private var animateChanges = false
  private var transitioning = false
  var onVisibilitySettled: (Bool) -> Void = { _ in }

  init(sidebar: Sidebar, detail: Detail, isVisible: Bool) {
    requestedVisible = isVisible
    sidebarHost = NSHostingController(rootView: sidebar)
    detailHost = NSHostingController(rootView: detail)
    sidebarItem = NSSplitViewItem(viewController: sidebarHost)
    super.init(nibName: nil, bundle: nil)
    sidebarHost.sizingOptions = []
    detailHost.sizingOptions = []
    splitView.isVertical = true
    splitView.dividerStyle = .thin
    sidebarItem.minimumThickness = 340
    // Visibility is owned by the toolbar/keyboard binding, not divider gestures.
    sidebarItem.canCollapse = false
    sidebarItem.canCollapseFromWindowResize = false
    sidebarItem.collapseBehavior = .preferResizingSiblingsWithFixedSplitView
    // Preserve its width when the window resizes, but yield to divider drags (490).
    sidebarItem.holdingPriority = NSLayoutConstraint.Priority(260)
    sidebarItem.isCollapsed = !isVisible
    let detailItem = NSSplitViewItem(viewController: detailHost)
    detailItem.minimumThickness = 380
    addSplitViewItem(sidebarItem)
    addSplitViewItem(detailItem)
  }

  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

  func setMaximumSidebarWidth(_ width: CGFloat) {
    sidebarItem.maximumThickness = max(sidebarItem.minimumThickness, width)
  }

  func setSidebarVisible(_ visible: Bool, animated: Bool) {
    requestedVisible = visible
    animateChanges = animated
    applyRequestedVisibility()
  }

  private func applyRequestedVisibility() {
    guard !transitioning, sidebarItem.isCollapsed == requestedVisible else { return }
    if animateChanges && view.window != nil {
      transitioning = true
      NSAnimationContext.runAnimationGroup { context in
        context.duration = 0.15
        context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        sidebarItem.animator().isCollapsed = !requestedVisible
      } completionHandler: { [weak self] in
        guard let self else { return }
        self.transitioning = false
        self.applyRequestedVisibility()
        if !self.transitioning { self.onVisibilitySettled(self.requestedVisible) }
      }
    } else {
      sidebarItem.isCollapsed = !requestedVisible
      view.layoutSubtreeIfNeeded()
      let settled = requestedVisible
      DispatchQueue.main.async { [weak self] in
        guard let self, !self.transitioning, self.requestedVisible == settled else { return }
        self.onVisibilitySettled(settled)
      }
    }
  }
}
