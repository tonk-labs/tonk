// swift-tools-version: 5.9
import PackageDescription

let package = Package(
  name: "Tonk",
  platforms: [.macOS("15.0")],
  products: [.executable(name: "Tonk", targets: ["Tonk"])],
  dependencies: [
    .package(url: "https://github.com/gonzalezreal/textual", from: "0.5.0"),
    .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.6.0"),
  ],
  targets: [
    .target(name: "HarnessCore"),
    .executableTarget(
      name: "Tonk",
      dependencies: [
        "HarnessCore", .product(name: "Textual", package: "textual"),
        .product(name: "Sparkle", package: "Sparkle"),
      ],
      resources: [.process("Assets.xcassets")]),
    .testTarget(name: "HarnessCoreTests", dependencies: ["HarnessCore"]),
    .testTarget(name: "TonkTests", dependencies: ["Tonk"]),
  ]
)
