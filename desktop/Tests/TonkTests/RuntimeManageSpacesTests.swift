import XCTest

@testable import Tonk

@MainActor
final class RuntimeManageSpacesTests: XCTestCase {
  private enum Failure: Error { case lostResponse, unconfirmed }

  func testAcceptedCreationIsRecoveredAfterSubmissionLosesItsResponse() async throws {
    let request = UUID().uuidString
    var submitted: [String] = []
    var receiptRequests: [String] = []
    var receipts: [String: (String, String)] = [:]
    let result = try await RuntimeModel.submitAndConfirmSpaceCreation {
      submitted.append(request)
      receipts[request] = ("created", "/space/did:key:created")
      throw Failure.lostResponse
    } confirm: {
      receiptRequests.append(request)
      return try XCTUnwrap(receipts[request])
    }
    XCTAssertEqual(result.0, "created")
    XCTAssertEqual(result.1, "/space/did:key:created")
    XCTAssertEqual(submitted, [request])
    XCTAssertEqual(receiptRequests, [request])
  }

  func testUnconfirmedCreationDoesNotResubmitOrReportSuccess() async {
    var submissions = 0
    var confirmations = 0
    do {
      _ = try await RuntimeModel.submitAndConfirmSpaceCreation {
        submissions += 1
        throw Failure.lostResponse
      } confirm: {
        confirmations += 1
        throw Failure.unconfirmed
      }
      XCTFail("An absent receipt must not report success")
    } catch Failure.unconfirmed {
      // The confirmation outcome, not the ambiguous transport error, is returned.
    } catch {
      XCTFail("Unexpected error: \(error)")
    }
    XCTAssertEqual(submissions, 1)
    XCTAssertEqual(confirmations, 1)
  }
}
