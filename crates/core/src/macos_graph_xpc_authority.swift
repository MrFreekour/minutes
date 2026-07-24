import Foundation
import Security

private let minutesDesktopSigningRequirement =
    #"anchor apple generic and certificate leaf[subject.OU] = "63TMLKT8HN" and (identifier "com.useminutes.desktop" or identifier "com.useminutes.desktop.dev")"#
private let minutesTeamSigningRequirement =
    #"anchor apple generic and certificate leaf[subject.OU] = "63TMLKT8HN""#

private func codeDirectoryHash(_ code: SecStaticCode) throws -> Data {
    var information: CFDictionary?
    guard SecCodeCopySigningInformation(
        code,
        SecCSFlags(),
        &information
    ) == errSecSuccess,
    let values = information as? [CFString: Any],
    let hash = values[kSecCodeInfoUnique] as? Data,
    hash.count == 20 else {
        throw POSIXError(.EACCES)
    }
    return hash
}

private func validatedCodeDirectoryHash(at path: String) throws -> Data {
    var staticCode: SecStaticCode?
    guard SecStaticCodeCreateWithPath(
        URL(fileURLWithPath: path) as CFURL,
        SecCSFlags(),
        &staticCode
    ) == errSecSuccess,
    let staticCode else {
        throw POSIXError(.EACCES)
    }
    let flags = SecCSFlags(
        rawValue: kSecCSCheckAllArchitectures
            | kSecCSStrictValidate
            | kSecCSRestrictSymlinks
    )
    guard SecStaticCodeCheckValidity(staticCode, flags, nil) == errSecSuccess else {
        throw POSIXError(.EACCES)
    }
    return try codeDirectoryHash(staticCode)
}

private func validateSealedAuthorityBundle(
    _ bundlePath: String,
    currentExecutablePath: String,
    runningParentCodeDirectoryHash: Data
) throws {
    var requirement: SecRequirement?
    guard SecRequirementCreateWithString(
        minutesDesktopSigningRequirement as CFString,
        SecCSFlags(),
        &requirement
    ) == errSecSuccess,
    let requirement else {
        throw POSIXError(.EACCES)
    }
    var staticCode: SecStaticCode?
    let createStatus = SecStaticCodeCreateWithPath(
        URL(fileURLWithPath: bundlePath) as CFURL,
        SecCSFlags(),
        &staticCode
    )
    guard createStatus == errSecSuccess, let staticCode else {
        throw POSIXError(.EACCES)
    }
    let flags = SecCSFlags(
        rawValue: kSecCSCheckAllArchitectures
            | kSecCSCheckNestedCode
            | kSecCSStrictValidate
            | kSecCSRestrictSymlinks
    )
    guard SecStaticCodeCheckValidity(
        staticCode,
        flags,
        requirement
    ) == errSecSuccess else {
        throw POSIXError(.EACCES)
    }

    // Tie the validated on-disk bundle back to the already-running parent.
    // This prevents replacing the whole package with an older legitimately
    // signed Minutes release before the helper request.
    var onDiskParent: SecStaticCode?
    guard SecStaticCodeCreateWithPath(
        URL(fileURLWithPath: currentExecutablePath) as CFURL,
        SecCSFlags(),
        &onDiskParent
    ) == errSecSuccess,
    let onDiskParent,
    try codeDirectoryHash(onDiskParent) == runningParentCodeDirectoryHash else {
        throw POSIXError(.EACCES)
    }
}

@_cdecl("minutes_current_process_is_trusted_distribution")
public func minutesCurrentProcessIsTrustedDistribution() -> Int32 {
    var requirement: SecRequirement?
    guard SecRequirementCreateWithString(
        minutesTeamSigningRequirement as CFString,
        SecCSFlags(),
        &requirement
    ) == errSecSuccess,
    let requirement else {
        return 0
    }
    var liveCode: SecCode?
    guard SecCodeCopySelf(SecCSFlags(), &liveCode) == errSecSuccess,
          let liveCode else {
        return 0
    }
    return SecCodeCheckValidity(
        liveCode,
        SecCSFlags(),
        requirement
    ) == errSecSuccess ? 1 : 0
}

/// Validate the signed application bundle that seals the embedded XPC
/// service's expected CDHash. Rust separately installs that exact CDHash as
/// the XPC peer code-signing requirement before the content-free handshake.
@_cdecl("minutes_validate_graph_authority_bundle")
public func minutesValidateGraphAuthorityBundle(
    _ authorityBundlePath: UnsafePointer<CChar>,
    _ currentExecutablePath: UnsafePointer<CChar>,
    _ runningParentCodeDirectoryHash: UnsafePointer<UInt8>,
    _ runningParentCodeDirectoryHashLength: Int
) -> Int32 {
    return autoreleasepool {
        do {
            guard runningParentCodeDirectoryHashLength == 20 else {
                throw POSIXError(.EINVAL)
            }
            let parentCodeDirectoryHash = Data(
                bytes: runningParentCodeDirectoryHash,
                count: runningParentCodeDirectoryHashLength
            )
            try validateSealedAuthorityBundle(
                String(cString: authorityBundlePath),
                currentExecutablePath: String(cString: currentExecutablePath),
                runningParentCodeDirectoryHash: parentCodeDirectoryHash
            )
            return 0
        } catch let error as POSIXError {
            return Int32(error.code.rawValue)
        } catch {
            return Int32(EACCES)
        }
    }
}

/// Return the exact CodeDirectory hash of a valid executable selected by the
/// authenticated parent. The audio XPC service compares this value with
/// `csops(CS_OPS_CDHASH)` for the suspended live child before the parent sends
/// a single private-audio byte.
@_cdecl("minutes_static_code_cdhash")
public func minutesStaticCodeDirectoryHash(
    _ executablePath: UnsafePointer<CChar>,
    _ output: UnsafeMutablePointer<UInt8>,
    _ outputLength: Int
) -> Int32 {
    return autoreleasepool {
        do {
            guard outputLength == 20 else {
                throw POSIXError(.EINVAL)
            }
            let hash = try validatedCodeDirectoryHash(at: String(cString: executablePath))
            hash.copyBytes(to: output, count: outputLength)
            return 0
        } catch let error as POSIXError {
            return Int32(error.code.rawValue)
        } catch {
            return Int32(EACCES)
        }
    }
}
