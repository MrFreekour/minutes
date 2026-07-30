// Narrow screenshot helper for automated UI verification.
//
// Why this exists as its own binary: TCC grants attach to an executable, so a
// purpose-built helper can hold Screen Recording while the rest of the machine
// does not. The alternative that keeps getting suggested is granting Screen
// Recording to sshd, which hands continuous screen access to anything that ever
// runs over SSH, forever. That is a far worse trade for the same capability.
//
// The security boundary is the target, not just the trigger. This helper can
// only capture windows owned by an allowlisted Minutes bundle, and it refuses
// everything else. Even fully compromised it cannot photograph a password
// manager, a banking session, or private messages, because it will not look at
// windows it does not own.
//
// Usage:
//   ui_shot <bundle-id> <output.png> [window-index]
//
// Exit codes:
//   0  captured
//   1  usage error
//   2  no window found for that bundle
//   3  bundle not allowlisted
//   4  capture failed (usually Screen Recording not granted)

import AppKit
import CoreGraphics
import Foundation

/// Bundle identifiers this helper is willing to photograph.
///
/// Deliberately a literal allowlist rather than a parameter or a prefix match:
/// the whole safety argument is that the set of capturable windows is fixed at
/// compile time and cannot be widened by whoever invokes the binary.
let allowedBundleIDs: Set<String> = [
    "com.useminutes.desktop",
    "com.useminutes.desktop.dev",
    "com.useminutes.desktop.uitest",
]

func fail(_ message: String, _ code: Int32) -> Never {
    FileHandle.standardError.write(Data(("ui_shot: " + message + "\n").utf8))
    exit(code)
}

let args = CommandLine.arguments
guard args.count >= 3 else {
    fail("usage: ui_shot <bundle-id> <output.png> [window-index]", 1)
}

let bundleID = args[1]
let outputPath = args[2]
let windowIndex = args.count >= 4 ? Int(args[3]) ?? 0 : 0

guard allowedBundleIDs.contains(bundleID) else {
    fail(
        "refusing to capture '\(bundleID)': not in the allowlist. This helper only "
            + "captures Minutes windows by design.",
        3
    )
}

// Resolve the bundle to running process ids. Matching on pid rather than window
// title avoids capturing an unrelated window that happens to share a name.
let pids = NSRunningApplication.runningApplications(withBundleIdentifier: bundleID)
    .map { $0.processIdentifier }
guard !pids.isEmpty else {
    fail("no running application for bundle '\(bundleID)'", 2)
}

guard
    let listing = CGWindowListCopyWindowInfo(
        [.optionOnScreenOnly, .excludeDesktopElements],
        kCGNullWindowID
    ) as? [[String: Any]]
else {
    fail("could not list windows", 4)
}

// Keep only on-screen windows owned by the resolved pids, largest first, so
// index 0 is the main window rather than a tooltip or a status item.
struct Candidate {
    let id: CGWindowID
    let area: CGFloat
}

var candidates: [Candidate] = []
for entry in listing {
    guard
        let ownerPID = entry[kCGWindowOwnerPID as String] as? pid_t,
        pids.contains(ownerPID),
        let windowNumber = entry[kCGWindowNumber as String] as? Int,
        let bounds = entry[kCGWindowBounds as String] as? [String: Any],
        let width = bounds["Width"] as? CGFloat,
        let height = bounds["Height"] as? CGFloat
    else { continue }
    // Skip degenerate windows (menu extras, offscreen shells).
    if width < 50 || height < 50 { continue }
    candidates.append(Candidate(id: CGWindowID(windowNumber), area: width * height))
}

candidates.sort { $0.area > $1.area }

guard !candidates.isEmpty else {
    fail(
        "'\(bundleID)' is running but has no capturable on-screen window. For a menu "
            + "bar app, open its window first.",
        2
    )
}
guard windowIndex < candidates.count else {
    fail("window-index \(windowIndex) out of range (\(candidates.count) windows)", 1)
}

let target = candidates[windowIndex].id

guard
    let image = CGWindowListCreateImage(
        .null,
        .optionIncludingWindow,
        target,
        [.boundsIgnoreFraming, .bestResolution]
    )
else {
    fail(
        "capture returned no image. This usually means Screen Recording is not granted "
            + "to this helper: System Settings > Privacy & Security > Screen & System "
            + "Audio Recording.",
        4
    )
}

// A capture with no pixels is what macOS hands back when the grant is missing
// but the call itself succeeds, so treat it as the same failure rather than
// writing an empty file that looks like success.
guard image.width > 0, image.height > 0 else {
    fail("capture produced an empty image; check the Screen Recording grant", 4)
}

let bitmap = NSBitmapImageRep(cgImage: image)
guard let png = bitmap.representation(using: .png, properties: [:]) else {
    fail("could not encode PNG", 4)
}

let outputURL = URL(fileURLWithPath: outputPath)
do {
    try png.write(to: outputURL, options: .atomic)
} catch {
    fail("could not write \(outputPath): \(error.localizedDescription)", 4)
}

print("captured \(image.width)x\(image.height) from \(bundleID) -> \(outputPath)")
