import AppKit
import CoreGraphics
import Darwin
import Foundation

func builds() -> [String] {
  let capacity = max(32, Int(proc_listallpids(nil, 0)) + 32)
  var pids = [pid_t](repeating: 0, count: capacity)
  let count = pids.withUnsafeMutableBytes {
    proc_listallpids($0.baseAddress, Int32($0.count))
  }
  let names: Set<String> = ["cargo", "cargo-clippy", "rustc", "clippy-driver"]
  return pids.prefix(max(0, Int(count))).compactMap { pid in
    var name = [CChar](repeating: 0, count: 64)
    guard proc_name(pid, &name, UInt32(name.count)) > 0 else { return nil }
    let value = String(cString: name)
    return names.contains(value) ? "\(value):\(pid)" : nil
  }
}

func inspect(_ pid: Int) -> [String: Any] {
  let sampled = Date().timeIntervalSince1970*1_000_000_000
  let session = CGSessionCopyCurrentDictionary() as? [String: Any] ?? [:]
  let windows = CGWindowListCopyWindowInfo(
    [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
  ) as? [[String: Any]] ?? []
  let visible = windows.filter {
    ($0[kCGWindowLayer as String] as? Int) == 0 &&
    ($0[kCGWindowAlpha as String] as? Double ?? 1) > 0
  }
  let screen = CGDisplayBounds(CGMainDisplayID())
  var result: [String: Any] = [
    "screen": ["x":screen.minX,"y":screen.minY,
               "width":screen.width,"height":screen.height],
    "sampled_unix_ns": sampled,
    "front": NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "",
    "front_pid": NSWorkspace.shared.frontmostApplication?.processIdentifier ?? -1,
    "pid": pid,
    "builds": builds(),
    "locked": session["CGSSessionScreenIsLocked"] as? Bool ?? false,
  ]
  if let index = visible.firstIndex(where: {
    ($0[kCGWindowOwnerPID as String] as? Int) == pid
  }), let bounds = visible[index][kCGWindowBounds as String] as? [String: Any],
     let rectangle = CGRect(dictionaryRepresentation: bounds as CFDictionary) {
    result["window"] = visible[index][kCGWindowBounds as String]
    result["occluded"] = visible.prefix(index).contains { window in
      guard let bounds = window[kCGWindowBounds as String] as? [String: Any],
            let other = CGRect(dictionaryRepresentation: bounds as CFDictionary) else {
        return false
      }
      return other.intersects(rectangle)
    }
    var displays = [CGDirectDisplayID](repeating: 0, count: 16)
    var count: UInt32 = 0
    CGGetActiveDisplayList(16, &displays, &count)
    for display in displays.prefix(Int(count)) {
      if CGDisplayBounds(display).contains(rectangle) {
        result["display"] = display
        result["hz"] = CGDisplayCopyDisplayMode(display)?.refreshRate ?? 0
        if let screen = NSScreen.screens.first(where: {
          ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")]
            as? NSNumber)?.uint32Value == display
        }) {
          result["max_hz"] = screen.maximumFramesPerSecond
        }
      }
    }
  }
  return result
}

let watch = CommandLine.arguments[1] == "--watch"
let target = CommandLine.arguments[watch ? 2 : 1]
repeat {
  autoreleasepool {
    let pid = watch ? Int((try? String(contentsOfFile: target,
      encoding: .utf8))?.trimmingCharacters(in: .whitespacesAndNewlines)
      ?? "-1") ?? -1 : Int(target) ?? -1
    if let data = try? JSONSerialization.data(withJSONObject: inspect(pid)),
       let text = String(data: data, encoding: .utf8) {
      print(text)
      fflush(stdout)
    }
  }
  if watch {
    RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.25))
  }
} while watch
