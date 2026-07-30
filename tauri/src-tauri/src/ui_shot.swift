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
//   ui_shot --serve                  watch for capture requests (LaunchAgent mode)
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
    /// Directory the agent watches for capture requests.
    static var requestDir: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".minutes/ui-shot/requests")
    }

    /// Directory the agent writes results to.
    static var responseDir: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".minutes/ui-shot/responses")
    }

    /// Watch for request files and fulfill them, until killed.
    ///
    /// Requests are one JSON file per capture: `{"bundle":"...","output":"...",
    /// "index":0}`. The response is written to `responses/<same-name>` as
    /// `{"ok":true,...}` or `{"ok":false,"error":"..."}`, and the request file is
    /// removed so it is never served twice.
    ///
    /// The allowlist still applies here. Running in the GUI session widens *who*
    /// can ask, not *what* can be captured, so a request for an unrelated bundle
    /// is refused exactly as it is on the command line.
    static func serve() async -> Never {
        let fm = FileManager.default
        for dir in [requestDir, responseDir] {
            try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        }
        // Owner-only: request and response paths name windows and file
        // locations, and nothing else on the machine needs to read them.
        for dir in [requestDir, responseDir] {
            try? fm.setAttributes([.posixPermissions: 0o700], ofItemAtPath: dir.path)
        }

        FileHandle.standardError.write(
            Data("ui_shot: serving requests from \(requestDir.path)\n".utf8)
        )

        while true {
            let entries =
                (try? fm.contentsOfDirectory(at: requestDir, includingPropertiesForKeys: nil))
                ?? []
            for entry in entries where entry.pathExtension == "json" {
                await fulfill(entry)
            }
            try? await Task.sleep(nanoseconds: 400_000_000)
        }
    }

    static func fulfill(_ request: URL) async {
        let fm = FileManager.default
        let name = request.lastPathComponent
        var result: [String: Any] = ["ok": false]

        defer {
            // Remove the request first so a crash mid-write cannot cause the
            // same request to be replayed forever.
            try? fm.removeItem(at: request)
            if let data = try? JSONSerialization.data(withJSONObject: result) {
                try? data.write(to: responseDir.appendingPathComponent(name), options: .atomic)
            }
        }

        guard
            let data = try? Data(contentsOf: request),
            let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let bundle = payload["bundle"] as? String,
            let output = payload["output"] as? String
        else {
            result["error"] = "malformed request"
            return
        }
        let index = payload["index"] as? Int ?? 0

        guard allowedBundleIDs.contains(bundle) else {
            result["error"] = "bundle '\(bundle)' is not in the allowlist"
            return
        }

        switch await capture(bundle: bundle, outputPath: output, windowIndex: index) {
        case .success(let described):
            result = ["ok": true, "detail": described, "output": output]
        case .failure(let message):
            result["error"] = message
        }
    }

    enum CaptureOutcome {
        case success(String)
        case failure(String)
    }

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

        // Serve mode. macOS attributes screen capture to the *responsible*
        // process, so invoking this binary over SSH attributes the request to
        // sshd, which is not granted, even though the binary itself is. And
        // `launchctl asuser` needs root. So the only way for a remote caller to
        // get a capture without granting sshd is for the helper to already be
        // running inside the GUI session and take requests. That is what a
        // LaunchAgent gives us.
        if args.count >= 2, args[1] == "--serve" {
            await serve()
        }

        guard args.count >= 3 else {
            fail(
                "usage: ui_shot <bundle-id> <output.png> [window-index] | ui_shot --check"
                    + " | ui_shot --serve",
                1
            )
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

        guard CGPreflightScreenCaptureAccess() else {
            fail(
                "Screen Recording is not granted for this invocation. The binary is "
                    + "\(args[0]). Note that macOS attributes capture to the responsible "
                    + "process, so running this over SSH is attributed to sshd and will fail "
                    + "even when the binary itself is granted. Use --serve via the LaunchAgent "
                    + "for remote captures.",
                5
            )
        }

        switch await capture(bundle: bundleID, outputPath: outputPath, windowIndex: windowIndex) {
        case .success(let described):
            print(described)
        case .failure(let message):
            fail(message, 4)
        }
    }

    /// Capture one allowlisted window to a PNG.
    ///
    /// Shared by the command line and serve paths so both enforce the same
    /// allowlist and produce the same output, rather than drifting.
    static func capture(
        bundle: String,
        outputPath: String,
        windowIndex: Int
    ) async -> CaptureOutcome {
        guard allowedBundleIDs.contains(bundle) else {
            return .failure("bundle '\(bundle)' is not in the allowlist")
        }

        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(
                true,
                onScreenWindowsOnly: true
            )
        } catch {
            return .failure("could not enumerate shareable content: \(error.localizedDescription)")
        }

        // Largest first, so index 0 is the main window rather than a tooltip or
        // status item.
        let candidates = content.windows
            .filter { window in
                window.owningApplication?.bundleIdentifier == bundle
                    && window.frame.width >= 50
                    && window.frame.height >= 50
            }
            .sorted { ($0.frame.width * $0.frame.height) > ($1.frame.width * $1.frame.height) }

        guard !candidates.isEmpty else {
            return .failure(
                "'\(bundle)' has no capturable on-screen window. For a menu bar app, open its "
                    + "window first."
            )
        }
        guard windowIndex < candidates.count else {
            return .failure(
                "window-index \(windowIndex) out of range (\(candidates.count) windows)"
            )
        }

        let target = candidates[windowIndex]
        let filter = SCContentFilter(desktopIndependentWindow: target)
        let config = SCStreamConfiguration()
        // Backing scale, so text in the result is legible rather than
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
            return .failure("capture failed: \(error.localizedDescription)")
        }

        guard image.width > 0, image.height > 0 else {
            return .failure("capture produced an empty image")
        }

        let bitmap = NSBitmapImageRep(cgImage: image)
        guard let png = bitmap.representation(using: .png, properties: [:]) else {
            return .failure("could not encode PNG")
        }

        do {
            try png.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
        } catch {
            return .failure("could not write \(outputPath): \(error.localizedDescription)")
        }

        let title = target.title ?? "(untitled)"
        return .success("captured \(image.width)x\(image.height) \"\(title)\" -> \(outputPath)")
    }
}
