// Input-source switcher for the IME check (Carbon TIS API).
//   swift tools/ime.swift current | find <substring> | enable <id> | select <id> | disable <id>
import Carbon
import Foundation

func sources(_ id: String?, all: Bool) -> [TISInputSource] {
    var filter: [String: Any] = [:]
    if let id { filter[kTISPropertyInputSourceID as String] = id }
    let list = TISCreateInputSourceList(filter as CFDictionary, all)?.takeRetainedValue() as? [TISInputSource]
    return list ?? []
}

func prop(_ s: TISInputSource, _ key: CFString) -> String {
    guard let p = TISGetInputSourceProperty(s, key) else { return "" }
    return Unmanaged<CFString>.fromOpaque(p).takeUnretainedValue() as String
}

let a = CommandLine.arguments
switch a[1] {
case "current":
    print(prop(TISCopyCurrentKeyboardInputSource().takeRetainedValue(), kTISPropertyInputSourceID))
case "find":
    for s in sources(nil, all: true) where prop(s, kTISPropertyInputSourceID).contains(a[2]) {
        print(prop(s, kTISPropertyInputSourceID))
    }
case "enable", "select", "disable":
    guard let s = sources(a[2], all: true).first else { print("not found"); exit(1) }
    let status: OSStatus
    switch a[1] {
    case "enable": status = TISEnableInputSource(s)
    case "select": status = TISSelectInputSource(s)
    default: status = TISDisableInputSource(s)
    }
    print("\(a[1]) \(a[2]) -> \(status)")
default:
    print("unknown")
}
