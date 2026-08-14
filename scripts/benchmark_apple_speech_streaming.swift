@preconcurrency import AVFAudio
import CoreMedia
import Foundation
import Speech

struct Event: Codable {
    let elapsedMs: UInt64
    let audioMs: Int
    let isFinal: Bool
    let text: String
}

actor EventLog {
    private var events: [Event] = []

    func append(_ event: Event) {
        events.append(event)
        fputs("event elapsed=\(event.elapsedMs)ms audio=\(event.audioMs)ms final=\(event.isFinal) text=\(event.text)\n", stderr)
    }

    func snapshot() -> [Event] { events }
}

enum BenchmarkError: Error, CustomStringConvertible {
    case failed(String)

    var description: String {
        switch self {
        case .failed(let message): return message
        }
    }
}

@available(macOS 26.0, *)
func readAndConvert(_ path: String, modules: [any SpeechModule]) async throws -> AVAudioPCMBuffer {
    let file = try AVAudioFile(forReading: URL(fileURLWithPath: path))
    guard let source = AVAudioPCMBuffer(
        pcmFormat: file.processingFormat,
        frameCapacity: AVAudioFrameCount(file.length)
    ) else {
        throw BenchmarkError.failed("could not allocate source audio")
    }
    try file.read(into: source)
    guard let targetFormat = await SpeechAnalyzer.bestAvailableAudioFormat(
        compatibleWith: modules,
        considering: source.format
    ) else {
        throw BenchmarkError.failed("no compatible audio format")
    }
    if source.format == targetFormat {
        return source
    }
    guard let converter = AVAudioConverter(from: source.format, to: targetFormat) else {
        throw BenchmarkError.failed("could not create audio converter")
    }
    let ratio = targetFormat.sampleRate / source.format.sampleRate
    let capacity = AVAudioFrameCount((Double(source.frameLength) * ratio).rounded(.up)) + 1
    guard let converted = AVAudioPCMBuffer(pcmFormat: targetFormat, frameCapacity: capacity) else {
        throw BenchmarkError.failed("could not allocate converted audio")
    }
    var supplied = false
    var conversionError: NSError?
    let status = converter.convert(to: converted, error: &conversionError) { _, outputStatus in
        if supplied {
            outputStatus.pointee = .endOfStream
            return nil
        }
        supplied = true
        outputStatus.pointee = .haveData
        return source
    }
    if status == .error {
        throw conversionError ?? BenchmarkError.failed("audio conversion failed")
    }
    return converted
}

func copyFrames(
    from source: AVAudioPCMBuffer,
    startFrame: AVAudioFramePosition,
    frameCount: AVAudioFrameCount
) throws -> AVAudioPCMBuffer {
    guard let chunk = AVAudioPCMBuffer(pcmFormat: source.format, frameCapacity: frameCount) else {
        throw BenchmarkError.failed("could not allocate audio chunk")
    }
    chunk.frameLength = frameCount
    let bytesPerFrame = Int(source.format.streamDescription.pointee.mBytesPerFrame)
    let buffers = UnsafeMutableAudioBufferListPointer(source.mutableAudioBufferList)
    let destinationBuffers = UnsafeMutableAudioBufferListPointer(chunk.mutableAudioBufferList)
    for index in 0..<buffers.count {
        guard let sourceData = buffers[index].mData, let destinationData = destinationBuffers[index].mData else {
            throw BenchmarkError.failed("audio buffer has no data")
        }
        memcpy(
            destinationData,
            sourceData.advanced(by: Int(startFrame) * bytesPerFrame),
            Int(frameCount) * bytesPerFrame
        )
        destinationBuffers[index].mDataByteSize = UInt32(Int(frameCount) * bytesPerFrame)
    }
    return chunk
}

@available(macOS 26.0, *)
func pacedInputs(_ audio: AVAudioPCMBuffer, chunkMs: Int) -> AsyncThrowingStream<AnalyzerInput, Error> {
    AsyncThrowingStream { continuation in
        Task {
            do {
                let framesPerChunk = max(
                    AVAudioFrameCount(audio.format.sampleRate * Double(chunkMs) / 1_000.0),
                    1
                )
                let timescale = max(Int32(audio.format.sampleRate.rounded()), 1)
                let started = DispatchTime.now().uptimeNanoseconds
                var frame: AVAudioFramePosition = 0
                while frame < AVAudioFramePosition(audio.frameLength) {
                    let remaining = AVAudioFramePosition(audio.frameLength) - frame
                    let count = min(framesPerChunk, AVAudioFrameCount(remaining))
                    let chunk = try copyFrames(from: audio, startFrame: frame, frameCount: count)
                    continuation.yield(
                        AnalyzerInput(
                            buffer: chunk,
                            bufferStartTime: CMTime(value: CMTimeValue(frame), timescale: timescale)
                        )
                    )
                    frame += AVAudioFramePosition(count)
                    if frame < AVAudioFramePosition(audio.frameLength) {
                        let targetElapsed = UInt64(
                            Double(frame) / audio.format.sampleRate * 1_000_000_000
                        )
                        let elapsed = DispatchTime.now().uptimeNanoseconds - started
                        if targetElapsed > elapsed {
                            try await Task.sleep(for: .nanoseconds(targetElapsed - elapsed))
                        }
                    }
                }
                continuation.finish()
            } catch {
                continuation.finish(throwing: error)
            }
        }
    }
}

@available(macOS 26.0, *)
func printSummary(
    audio: AVAudioPCMBuffer,
    chunkMs: Int,
    presetName: String,
    events: [Event]
) throws {
    let useful = events.filter { !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    let usefulCadences = zip(useful.dropFirst(), useful).map { Int($0.0.elapsedMs - $0.1.elapsedMs) }
    let summary: [String: Any] = [
        "audioDurationMs": Int(Double(audio.frameLength) / audio.format.sampleRate * 1_000),
        "chunkMs": chunkMs,
        "preset": presetName,
        "eventCount": events.count,
        "firstUsefulMs": useful.first?.elapsedMs as Any,
        "maxUsefulCadenceMs": usefulCadences.max() as Any,
        "events": try JSONSerialization.jsonObject(with: JSONEncoder().encode(events)),
    ]
    let json = try JSONSerialization.data(withJSONObject: summary, options: [.prettyPrinted, .sortedKeys])
    print(String(decoding: json, as: UTF8.self))
}

@available(macOS 26.0, *)
func runSpeech(audioPath: String, chunkMs: Int) async throws {
    let transcriber = SpeechTranscriber(
        locale: Locale(identifier: "en-US"),
        transcriptionOptions: [],
        reportingOptions: [.fastResults],
        attributeOptions: [.audioTimeRange]
    )
    let modules: [any SpeechModule] = [transcriber]
    if let request = try await AssetInventory.assetInstallationRequest(supporting: modules) {
        try await request.downloadAndInstall()
    }
    let audio = try await readAndConvert(audioPath, modules: modules)
    let analyzer = SpeechAnalyzer(modules: modules)
    let started = DispatchTime.now().uptimeNanoseconds
    let log = EventLog()
    let resultTask = Task {
        for try await result in transcriber.results {
            let elapsed = (DispatchTime.now().uptimeNanoseconds - started) / 1_000_000
            let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty {
                await log.append(Event(
                    elapsedMs: elapsed,
                    audioMs: Int(CMTimeGetSeconds(CMTimeRangeGetEnd(result.range)) * 1_000),
                    isFinal: result.isFinal,
                    text: text
                ))
            }
        }
    }
    if let lastSample = try await analyzer.analyzeSequence(pacedInputs(audio, chunkMs: chunkMs)) {
        try await analyzer.finalizeAndFinish(through: lastSample)
    } else {
        await analyzer.cancelAndFinishNow()
    }
    try await resultTask.value
    try printSummary(audio: audio, chunkMs: chunkMs, presetName: "speech-fast", events: await log.snapshot())
}

@available(macOS 26.0, *)
func run() async throws {
    guard CommandLine.arguments.count >= 2 else {
        throw BenchmarkError.failed("usage: benchmark_apple_speech_streaming AUDIO [chunk-ms] [preset|speech]")
    }
    let audioPath = CommandLine.arguments[1]
    let chunkMs = CommandLine.arguments.count > 2 ? Int(CommandLine.arguments[2]) ?? 100 : 100
    let locale = Locale(identifier: "en-US")
    let presetName = CommandLine.arguments.count > 3 ? CommandLine.arguments[3] : "progressive-short"
    if presetName == "speech" {
        try await runSpeech(audioPath: audioPath, chunkMs: chunkMs)
        return
    }
    let transcriber: DictationTranscriber
    if presetName == "volatile-no-punctuation" {
        transcriber = DictationTranscriber(
            locale: locale,
            contentHints: [.shortForm],
            transcriptionOptions: [],
            reportingOptions: [.volatileResults, .frequentFinalization],
            attributeOptions: [.audioTimeRange]
        )
    } else {
        let preset: DictationTranscriber.Preset
        switch presetName {
        case "phrase": preset = .phrase
        case "short": preset = .shortDictation
        case "progressive-long": preset = .progressiveLongDictation
        case "long": preset = .longDictation
        default: preset = .progressiveShortDictation
        }
        transcriber = DictationTranscriber(locale: locale, preset: preset)
    }
    let modules: [any SpeechModule] = [transcriber]
    if let request = try await AssetInventory.assetInstallationRequest(supporting: modules) {
        try await request.downloadAndInstall()
    }
    let audio = try await readAndConvert(audioPath, modules: modules)
    let analyzer = SpeechAnalyzer(modules: modules)
    let started = DispatchTime.now().uptimeNanoseconds
    let log = EventLog()
    let resultTask = Task {
        for try await result in transcriber.results {
            let elapsed = (DispatchTime.now().uptimeNanoseconds - started) / 1_000_000
            let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty {
                await log.append(Event(
                    elapsedMs: elapsed,
                    audioMs: Int(CMTimeGetSeconds(CMTimeRangeGetEnd(result.range)) * 1_000),
                    isFinal: result.isFinal,
                    text: text
                ))
            }
        }
    }
    if let lastSample = try await analyzer.analyzeSequence(pacedInputs(audio, chunkMs: chunkMs)) {
        try await analyzer.finalizeAndFinish(through: lastSample)
    } else {
        await analyzer.cancelAndFinishNow()
    }
    try await resultTask.value
    let events = await log.snapshot()
    try printSummary(audio: audio, chunkMs: chunkMs, presetName: presetName, events: events)
}

@main
struct Main {
    static func main() async {
        guard #available(macOS 26.0, *) else {
            fputs("macOS 26 or newer is required\n", stderr)
            exit(2)
        }
        do {
            try await run()
        } catch {
            fputs("benchmark failed: \(error)\n", stderr)
            exit(1)
        }
    }
}
