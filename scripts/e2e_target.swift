// Throwaway paste target for end-to-end tests: a focused text view that prints what it received.
// Usage: e2e_target <seconds>   (prints the text view's contents to stdout, then quits)
import AppKit

let seconds = Double(CommandLine.arguments.dropFirst().first ?? "15") ?? 15
let app = NSApplication.shared
app.setActivationPolicy(.regular)

// ⌘V only reaches the text view through a Paste menu item's key equivalent.
let mainMenu = NSMenu()
let editItem = NSMenuItem()
editItem.submenu = NSMenu(title: "Edit")
editItem.submenu!.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
mainMenu.addItem(NSMenuItem())
mainMenu.addItem(editItem)
app.mainMenu = mainMenu

let window = NSWindow(contentRect: NSRect(x: 200, y: 200, width: 520, height: 220),
                      styleMask: [.titled], backing: .buffered, defer: false)
window.title = "Murmur E2E target"
let textView = NSTextView(frame: window.contentView!.bounds)
textView.isRichText = false
window.contentView!.addSubview(textView)
window.makeKeyAndOrderFront(nil)
window.makeFirstResponder(textView)
app.activate(ignoringOtherApps: true)
// E2E_FULLSCREEN=1 puts the target in its own full-screen Space (checks the pill floats over it).
if ProcessInfo.processInfo.environment["E2E_FULLSCREEN"] == "1" {
    window.styleMask.insert(.resizable)
    window.collectionBehavior.insert(.fullScreenPrimary)
    window.toggleFullScreen(nil)
}

// Optional scripted "user correction": e2e_target <seconds> <from> <to> <at-seconds>
let args = CommandLine.arguments
if args.count >= 5, let at = Double(args[4]) {
    DispatchQueue.main.asyncAfter(deadline: .now() + at) {
        let range = (textView.string as NSString).range(of: args[2])
        if range.location != NSNotFound {
            textView.insertText(args[3], replacementRange: range)
        } else {
            FileHandle.standardError.write("e2e_target: '\(args[2])' not found to correct\n".data(using: .utf8)!)
        }
    }
}

DispatchQueue.main.asyncAfter(deadline: .now() + seconds) {
    print(textView.string)
    app.terminate(nil)
}
app.run()
