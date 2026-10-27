import AppKit
import AVFoundation
import CoreMedia
import Foundation
import ScreenCaptureKit

final class Recorder: NSObject, SCStreamOutput {
  let writer: AVAssetWriter
  let input: AVAssetWriterInput
  let queue = DispatchQueue(label: "vpdf.demo.frames")
  var first: CMTime?
  var last: CMSampleBuffer?
  var frames = 0
  var drops = 0
  var lastSource: CMTime?
  var lastWritten: CMTime?
  var gaps = 0
  var statuses: [String: Int] = [:]

  init(url: URL, width: Int, height: Int) throws {
    writer = try AVAssetWriter(outputURL: url, fileType: .mov)
    input = AVAssetWriterInput(mediaType: .video, outputSettings: [
      AVVideoCodecKey: AVVideoCodecType.h264,
      AVVideoWidthKey: width,
      AVVideoHeightKey: height,
      AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: 24_000_000],
    ])
    input.expectsMediaDataInRealTime = true
    writer.add(input)
    super.init()
    guard writer.startWriting() else {
      throw writer.error ?? NSError(domain: "vpdf.demo.writer", code: 1)
    }
  }

  func stream(_ stream: SCStream, didOutputSampleBuffer buffer: CMSampleBuffer,
              of type: SCStreamOutputType) {
    guard type == .screen, buffer.isValid else { return }
    guard let attachments = CMSampleBufferGetSampleAttachmentsArray(buffer,
      createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
      let raw = attachments.first?[.status] as? Int else { return }
    statuses[String(raw), default: 0] += 1
    let pts = buffer.presentationTimeStamp
    if let previous = lastSource {
      gaps += max(0, Int(((pts.seconds - previous.seconds) * 30).rounded()) - 1)
    }
    lastSource = pts
    guard raw == SCFrameStatus.complete.rawValue || raw == SCFrameStatus.idle.rawValue else { return }
    var sample = buffer
    if raw == SCFrameStatus.idle.rawValue {
      guard let previous = last else { return }
      var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 30),
        presentationTimeStamp: pts, decodeTimeStamp: .invalid)
      var repeated: CMSampleBuffer?
      guard CMSampleBufferCreateCopyWithNewTiming(allocator: kCFAllocatorDefault,
        sampleBuffer: previous, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
        sampleBufferOut: &repeated) == noErr, let copy = repeated else { drops += 1; return }
      sample = copy
    }
    if first == nil {
      first = pts
      writer.startSession(atSourceTime: pts)
      let row: [String: Any] = ["ready": true,
        "first_frame_monotonic": pts.seconds,
        "callback_monotonic": ProcessInfo.processInfo.systemUptime]
      if let data = try? JSONSerialization.data(withJSONObject: row),
         let text = String(data: data, encoding: .utf8) {
        print(text)
        fflush(stdout)
      }
    }
    guard input.isReadyForMoreMediaData else { drops += 1; return }
    if input.append(sample) {
      frames += 1
      last = sample
      lastWritten = pts
    } else {
      drops += 1
    }
  }

  func finish() async throws -> [String: Any] {
    let elapsed: Double = queue.sync {
      guard let first = first, let last = last else { return 0 }
      let ending = CMTime(seconds: ProcessInfo.processInfo.systemUptime,
        preferredTimescale: 1_000_000_000)
      var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 30),
        presentationTimeStamp: ending, decodeTimeStamp: .invalid)
      var final: CMSampleBuffer?
      let result = CMSampleBufferCreateCopyWithNewTiming(allocator: kCFAllocatorDefault,
        sampleBuffer: last, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
        sampleBufferOut: &final)
      let deadline = Date(timeIntervalSinceNow: 2)
      while !input.isReadyForMoreMediaData && Date() < deadline { usleep(1000) }
      if result == noErr, let final = final, input.isReadyForMoreMediaData {
        if !input.append(final) { drops += 1 }
      } else { drops += 1 }
      writer.endSession(atSourceTime: ending)
      input.markAsFinished()
      return ending.seconds - first.seconds
    }
    await writer.finishWriting()
    guard writer.status == .completed, elapsed > 0 else {
      throw writer.error ?? NSError(domain: "vpdf.demo.empty", code: 1)
    }
    guard let first = first, let lastWritten = lastWritten,
          lastWritten.seconds > first.seconds else {
      throw NSError(domain: "vpdf.demo.frames", code: 1)
    }
    return ["frames": frames, "writer_dropped_frames": drops,
      "source_dropped_frames": NSNull(), "frame_status_counts": statuses,
      "first_frame_monotonic": first.seconds, "duration": elapsed,
      "actual_captured_fps": Double(frames - 1) / (lastWritten.seconds - first.seconds), "source_delivery_gap_slots": gaps, "requested_fps": 30,
      "source_drop_note": "Source drop counts are not exposed by ScreenCaptureKit."]
  }
}

@main struct Main {
  @MainActor static func main() async throws {
    NSApplication.shared.setActivationPolicy(.prohibited)
    let args = CommandLine.arguments
    guard args.count == 5, let identifier = UInt32(args[1]),
      let seconds = Double(args[2]), seconds > 0, seconds <= 40 else {
      throw NSError(domain: "vpdf.demo.arguments", code: 1)
    }
    let content = try await SCShareableContent.excludingDesktopWindows(false,
      onScreenWindowsOnly: true)
    guard let window = content.windows.first(where: { $0.windowID == identifier }) else {
      throw NSError(domain: "vpdf.demo.window", code: 1)
    }
    let config = SCStreamConfiguration()
    config.width = Int(window.frame.width)
    config.height = Int(window.frame.height)
    config.minimumFrameInterval = CMTime(value: 1, timescale: 30)
    config.queueDepth = 5
    config.showsCursor = false
    config.capturesAudio = false
    config.ignoreShadowsSingleWindow = true
    config.captureResolution = .nominal
    let recorder = try Recorder(url: URL(fileURLWithPath: args[3]),
      width: config.width, height: config.height)
    let filter = SCContentFilter(desktopIndependentWindow: window)
    let stream = SCStream(filter: filter, configuration: config, delegate: nil)
    try stream.addStreamOutput(recorder, type: .screen,
      sampleHandlerQueue: recorder.queue)
    try await stream.startCapture()
    try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
    try await stream.stopCapture()
    let report = try await recorder.finish()
    try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
      .write(to: URL(fileURLWithPath: args[4]), options: .atomic)
  }
}
