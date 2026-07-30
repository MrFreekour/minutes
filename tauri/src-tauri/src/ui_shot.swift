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
// Uses ScreenCaptureKit rather than CGWindowListCreateImage: the latter was
// deprecated in macOS 14 and returns nil for window captures on current systems
// even when Screen Recording is granted, which reads as a permission failure
// when it is actually an API failure.
//
// Usage:
//   ui_shot <bundle-id> <output.png> [window-index]
//   ui_shot --check                  report whether the grant is held
//
// Exit codes:
//   0  captured (or --check with the grant held)
//   1  usage error
//   2  no window found for that bundle
//   3  bundle not allowlisted
//   4  capture failed
//   5  Screen Recording not granted to this binary

import AppKit
import CoreGraphics
import Foundation
import ScreenCaptureKit

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

@main
struct UIShot {
    static func main() async {
        let args = CommandLine.arguments

        // Permission probe. Separate from a capture so a caller can distinguish
        // "not granted" from "granted but something else went wrong" without
        // needing a window open.
        if args.count >= 2, args[1] == "--check" {
            let granted = CGPreflightScreenCaptureAccess()
            print("screen-recording-granted: \(granted)")
            print("binary: \(args[0])")
            exit(granted ? 0 : 5)
        }

        guard args.count >= 3 else {
            fail("usage: ui_shot <bundle-id> <output.png> [window-index] | ui_shot --check", 1)
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

        // Check the grant before doing anything else, so a missing permission
        // reports as itself rather than as an empty capture.
        guard CGPreflightScreenCaptureAccess() else {
            fail(
                "Screen Recording is not granted to this binary (\(args[0])). Add it under "
                    + "System Settings > Privacy & Security > Screen & System Audio Recording. "
                    + "If it is already listed, remove the entry with the minus button and "
                    + "re-add it: a stale record for an older build stays listed but no longer "
                    + "matches.",
                5
            )
        }

        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(
                true,
                onScreenWindowsOnly: true
            )
        } catch {
            fail("could not enumerate shareable content: \(error.localizedDescription)", 4)
        }

        // Filter to windows owned by the allowlisted bundle, largest first so
        // index 0 is the main window rather than a tooltip or status item.
        let candidates = content.windows
            .filter { window in
                window.owningApplication?.bundleIdentifier == bundleID
                    && window.frame.width >= 50
                    && window.frame.height >= 50
            }
            .sorted { ($0.frame.width * $0.frame.height) > ($1.frame.width * $1.frame.height) }

        guard !candidates.isEmpty else {
            fail(
                "'\(bundleID)' has no capturable on-screen window. For a menu bar app, open "
                    + "its window first.",
                2
            )
        }
        guard windowIndex < candidates.count else {
            fail("window-index \(windowIndex) out of range (\(candidates.count) windows)", 1)
        }

        let target = candidates[windowIndex]
        let filter = SCContentFilter(desktopIndependentWindow: target)
        let config = SCStreamConfiguration()
        // Capture at backing scale so text is legible in the result rather than
        // downsampled to logical points.
        config.width = Int(target.frame.width * 2)
        config.height = Int(target.frame.height * 2)
        config.showsCursor = false

        let image: CGImage
        do {
            image = try await SCScreenshotManager.captureImage(
                contentFilter: filter,
                configuration: config
            )
        } catch {
            fail("capture failed: \(error.localizedDescription)", 4)
        }

        guard image.width > 0, image.height > 0 else {
            fail("capture produced an empty image", 4)
        }

        let bitmap = NSBitmapImageRep(cgImage: image)
        guard let png = bitmap.representation(using: .png, properties: [:]) else {
            fail("could not encode PNG", 4)
        }

        do {
            try png.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
        } catch {
            fail("could not write \(outputPath): \(error.localizedDescription)", 4)
        }

        let title = target.title ?? "(untitled)"
        print("captured \(image.width)x\(image.height) \"\(title)\" -> \(outputPath)")
    }
}
