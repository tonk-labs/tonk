import Foundation
import HarnessCore

@MainActor
extension HarnessModel {
  func dismissSpaceProposal() {
    guard !busy, !creatingSpace else { return }
    var next = saved
    next.conversation.spaceProposal = nil
    do {
      try store.save(next)
      saved = next
      spaceCreationError = nil
    } catch { spaceCreationError = error.localizedDescription }
  }

  func acceptSpaceProposal() async {
    guard !busy, !creatingSpace, canSend, saved.conversation.space == nil,
      var proposal = saved.conversation.spaceProposal, let runtime
    else { return }
    creatingSpace = true
    spaceCreationError = nil
    defer { creatingSpace = false }
    do {
      let submit = !proposal.submitted
      if submit {
        proposal = try await runtime.prepareSpaceCreation(proposal)
        proposal.submitted = true
        var next = saved
        next.conversation.spaceProposal = proposal
        try store.save(next)
        saved = next
      }
      let space = try await runtime.createSpace(proposal, submit: submit)
      var next = saved
      try next.conversation.attachCreatedSpace(space, proposalID: proposal.id)
      try store.save(next)
      saved = next
      resumed = false
      runtime.openSpace(space)
      // Let the hosted worker settle before the agent's first space tool.
      for _ in 0..<60 {
        if !runtime.loading { break }
        try await Task.sleep(for: .seconds(1))
      }
      creatingSpace = false
      await send(
        "I accepted the space proposal. Tonk created and attached ‘\(space.title)’ to this conversation. Continue my original request in this space.",
        showsUserMessage: false
      )
    } catch {
      if saved.conversation.spaceProposal == nil {
        self.error = error.localizedDescription
      } else {
        spaceCreationError = error.localizedDescription
      }
    }
  }
}
