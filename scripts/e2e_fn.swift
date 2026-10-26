// End-to-end driver: presses Murmur's hotkeys at the HID level while `say` speaks into the room,
// so the event tap, microphone capture, transcription and paste all run for real.
// Usage: e2e_fn [--ptt | --handsfree | --cancel] "phrase to speak"
import CoreGraphics
import Foundation

let src = CGEventSource(stateID: .hidSystemState)

func post(_ key: CGKeyCode, down: Bool, flags: CGEventFlags = [], flagsChanged: Bool = false) {
    let ev = CGEvent(keyboardEventSource: src, virtualKey: key, keyDown: down)!
    if flagsChanged { ev.type = .flagsChanged }
    ev.flags = flags
    ev.post(tap: .cghidEventTap)
    Thread.sleep(forTimeInterval: 0.05)
}

func fn(_ down: Bool) { post(63, down: down, flags: down ? .maskSecondaryFn : [], flagsChanged: true) }
func fnSpace() {
    fn(true)
    post(49, down: true, flags: .maskSecondaryFn)
    post(49, down: false, flags: .maskSecondaryFn)
    fn(false)
}
func speak(_ phrase: String) {
    let say = Process()
    say.executableURL = URL(fileURLWithPath: "/usr/bin/say")
    say.arguments = [phrase]
    try! say.run()
    say.waitUntilExit()
}

var args = Array(CommandLine.arguments.dropFirst())
let mode = args.first?.hasPrefix("--") == true ? args.removeFirst() : "--ptt"
let phrase = args.joined(separator: " ")

switch mode {
case "--handsfree":
    fnSpace()  // start hands-free; Fn is released while recording continues
    Thread.sleep(forTimeInterval: 0.4)
    speak(phrase)
    Thread.sleep(forTimeInterval: 0.5)
    fnSpace()  // finish and paste
case "--cancel":
    fn(true)
    Thread.sleep(forTimeInterval: 0.4)
    speak(phrase)
    post(53, down: true, flags: .maskSecondaryFn)  // Esc while still holding Fn
    post(53, down: false, flags: .maskSecondaryFn)
    fn(false)
default:
    fn(true)
    Thread.sleep(forTimeInterval: 0.4)  // let the recorder open the mic before speech starts
    speak(phrase)
    Thread.sleep(forTimeInterval: 0.5)
    fn(false)
}
