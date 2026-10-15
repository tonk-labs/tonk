import Foundation
import HarnessCore
import XCTest

@testable import Tonk

@MainActor
final class HarnessClaudeTests: XCTestCase {
  func testClaudeStreamsAndRecoversExpiredLoginWithoutUsingAPIClient() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let executable = directory.appendingPathComponent("claude")
    try """
    #!/usr/bin/python3
    import sys,json,os
    if sys.argv[1:3] == ['auth','status']:
        print(json.dumps({'loggedIn':not os.path.exists(os.path.join(os.path.dirname(__file__),'signed-out')),'authMethod':'claude.ai'}),flush=True)
    elif sys.argv[1:3] == ['auth','logout']:
        open(os.path.join(os.path.dirname(__file__),'signed-out'),'w').close()
    elif '--input-format' in sys.argv:
        request=json.loads(sys.stdin.read())
        print(json.dumps({'type':'control_response','response':{'subtype':'success','request_id':request['request_id'],'response':{'models':[{'value':'sonnet','displayName':'Sonnet from CLI','description':'Fixture model'}]}}}),flush=True)
    else:
        prompt=sys.stdin.read()
        if prompt == 'expired':
            print(json.dumps({'type':'result','subtype':'error_during_execution','is_error':True,'errors':['Failed to authenticate: OAuth session expired']}),flush=True)
        else:
            print(json.dumps({'type':'assistant','message':{'id':'fixture','content':[{'type':'text','text':'Claude response'}]}}),flush=True)
            print(json.dumps({'type':'result','subtype':'success','is_error':False}),flush=True)
    """.write(to: executable, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
    let api = APIModelClient { _, _, _ in
      XCTFail("Claude subscription must not call the API-key client")
      return APIMessage(role: "assistant")
    }
    let model = HarnessModel(
      directory: directory, apiClient: api, claudeClient: ClaudeCodeClient(executable: executable))
    defer { model.shutdown() }
    let signedOut = directory.appendingPathComponent("signed-out")
    try Data().write(to: signedOut)
    try await model.configureProvider(
      .claude, connection: ModelConnection(provider: .claude), key: "")
    XCTAssertFalse(model.signedIn)
    XCTAssertTrue(model.claudeModels.isEmpty)
    try FileManager.default.removeItem(at: signedOut)
    await model.connect()
    XCTAssertTrue(model.canSend)
    XCTAssertEqual(model.claudeModels.map(\.name), ["Sonnet from CLI"])
    XCTAssertNil(model.claudeModelsError)
    await model.send("hello")
    await model.apiTask?.value
    XCTAssertEqual(model.saved.conversation.lastTurnStatus, "completed")
    XCTAssertEqual(model.saved.conversation.claudeSessionStarted, true)
    XCTAssertTrue(model.saved.conversation.messages.contains { $0.text == "Claude response" })
    let transcript = model.saved.conversation
    await model.signOut()
    XCTAssertFalse(model.signedIn)
    XCTAssertFalse(model.canSend)
    XCTAssertTrue(model.claudeModels.isEmpty)
    XCTAssertTrue(FileManager.default.fileExists(atPath: signedOut.path))
    XCTAssertEqual(model.saved.conversation, transcript)
    try FileManager.default.removeItem(at: signedOut)
    await model.connect()
    XCTAssertTrue(model.signedIn)
    model.newConversation()
    await model.send("expired")
    await model.apiTask?.value
    XCTAssertFalse(model.signedIn)
    XCTAssertNil(model.saved.conversation.threadID)
    XCTAssertEqual(model.saved.conversation.lastTurnStatus, "failed")
    XCTAssertTrue(model.error?.contains("Sign in with Claude") == true)
    try await model.configureProvider(
      .disabled, connection: ModelConnection(provider: .disabled), key: "")
    XCTAssertFalse(model.canSend)
  }
}
