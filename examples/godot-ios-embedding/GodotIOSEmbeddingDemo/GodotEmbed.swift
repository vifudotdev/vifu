//
//  GodotEmbed.swift
//  GodotIOSEmbeddingDemo
//
//  Embeds libgodot using the ios_sample pattern from libgodot/samples/ios_sample/.
//  Defines VifuGodotApp (instance lifecycle), UIVifuGodotView (metal surface
//  + touch forwarding + CADisplayLink), and VifuGodotView (UIViewRepresentable).
//
//  Supports true stop/restart: call godotApp.boot(packFile:) to destroy the current
//  instance and create a fresh one, then replace the @State var in ContentView to
//  force SwiftUI to mount a new UIVifuGodotView and start it.
//

#if canImport(SwiftGodot) && canImport(SwiftGodotKit)

#if os(iOS)
import UIKit
import SwiftUI
import QuartzCore
@preconcurrency import SwiftGodot
@preconcurrency import SwiftGodotKit
import OSLog

private extension Logger {
    static let godot = Logger(
        subsystem: "dev.vifu.godot-ios-embedding",
        category: "Godot"
    )
}

// MARK: - VifuGodotApp

/// Wraps a single GodotInstance.  Create → boot(packFile:) / boot(packPath:) → show VifuGodotView.
/// To restart with a different PCK, set a new VifuGodotApp in ContentView @State
/// and the previous UIVifuGodotView's removeFromSuperview() will destroy the old
/// instance automatically.
///
/// PCK-switch ordering with dlopen:
///   boot() calls GodotInstance.create() (dlopen) immediately, matching original timing.
///   Then self.app = newApp triggers SwiftUI: old view removeFromSuperview() → dlclose,
///   new view didMoveToSuperview() → startGodotInstance() → inst.start().
///   With RTLD_LOCAL, the new libgodot load is isolated from the old one, so brief
///   coexistence between boot() and removeFromSuperview() is safe.
@MainActor
final class VifuGodotApp: ObservableObject {
    @Published private(set) var instance: GodotInstance?

    let maxTouchCount = 32
    var touches: [UITouch?] = Array(repeating: nil, count: 32)

    /// Create a Godot instance from the named pack file (no .pck extension).
    /// Only call once per VifuGodotApp object; use a fresh object to restart.
    func boot(packFile: String) {
        guard instance == nil else {
            Logger.godot.warning("[GodotEmbed] boot() called on an already-booted app — ignoring")
            return
        }
        Logger.godot.info("[GodotEmbed] boot() for pack: \(packFile)")
        let resourcePath = Bundle.main.resourcePath ?? "."
        instance = GodotInstance.create(args: [
            "--main-pack", "\(resourcePath)/\(packFile).pck",
            "--rendering-driver", "metal",
            "--rendering-method", "mobile",
            "--display-driver", "embedded",
        ])
        if instance == nil {
            Logger.godot.error("[GodotEmbed] GodotInstance.create() returned nil for pack: \(packFile)")
        } else {
            Logger.godot.info("[GodotEmbed] GodotInstance created for pack: \(packFile)")
        }
    }

    /// Create a Godot instance from an absolute .pck path on disk.
    func boot(packPath: String) {
        guard instance == nil else {
            Logger.godot.warning("[GodotEmbed] boot(packPath:) called on an already-booted app — ignoring")
            return
        }
        Logger.godot.info("[GodotEmbed] boot() for pack path: \(packPath)")
        instance = GodotInstance.create(args: [
            "--main-pack", packPath,
            "--rendering-driver", "metal",
            "--rendering-method", "mobile",
            "--display-driver", "embedded",
        ])
        if instance == nil {
            Logger.godot.error("[GodotEmbed] GodotInstance.create() returned nil for pack path: \(packPath)")
        } else {
            Logger.godot.info("[GodotEmbed] GodotInstance created for pack path: \(packPath)")
        }
    }

    // Called by UIVifuGodotView.removeFromSuperview() after destroying the C instance.
    func markInstanceDestroyed() {
        instance = nil
    }

    // MARK: Touch ID pool

    func getTouchId(touch: UITouch) -> Int {
        var first = -1
        for i in 0..<maxTouchCount {
            if first == -1, touches[i] == nil { first = i; continue }
            if touches[i] == touch { return i }
        }
        if first != -1 { touches[first] = touch; return first }
        return -1
    }

    func removeTouchId(id: Int) {
        touches[id] = nil
    }

    func touchIndex(for touch: UITouch) -> Int {
        for i in 0..<maxTouchCount { if touches[i] == touch { return i } }
        return -1
    }
}

// MARK: - UIVifuGodotView

/// UIView subclass that owns the CAMetalLayer rendering surface and CADisplayLink
/// for the Godot main loop iteration.  Exactly follows the ios_sample pattern.
class UIVifuGodotView: UIView {

    var renderingLayer: CAMetalLayer!
    private var displayLink: CADisplayLink?
    private var embedded: DisplayServerEmbedded?

    var godotApp: VifuGodotApp?
    private var hasStarted = false

    /// Called when a left-edge swipe is detected. Set by ContentView to exit the active game.
    var onEdgeSwipeExit: (() -> Void)?

    // MARK: Lifecycle

    override init(frame: CGRect) { super.init(frame: frame) }
    required init?(coder: NSCoder) { super.init(coder: coder) }

    private func commonInit() {
        renderingLayer = CAMetalLayer()
        let size = max(UIScreen.main.bounds.size.width, UIScreen.main.bounds.size.height)
        renderingLayer.frame.size = CGSize(width: size, height: size)
        renderingLayer.contentsScale = contentScaleFactor
        layer.addSublayer(renderingLayer)

        // Left-edge swipe to exit game — uses UIScreenEdgePanGestureRecognizer
        // which ONLY activates from the screen edge and doesn't steal normal drags.
        let edgePan = UIScreenEdgePanGestureRecognizer(target: self, action: #selector(handleEdgeSwipe(_:)))
        edgePan.edges = .left
        edgePan.cancelsTouchesInView = false
        addGestureRecognizer(edgePan)
    }

    @objc private func handleEdgeSwipe(_ gesture: UIScreenEdgePanGestureRecognizer) {
        guard gesture.state == .ended else { return }
        let translation = gesture.translation(in: self)
        if translation.x > 80 {
            onEdgeSwipeExit?()
        }
    }

    deinit {
        MainActor.assumeIsolated {
            renderingLayer?.removeFromSuperlayer()
        }
    }

    override func layoutSubviews() {
        renderingLayer.frame = bounds
        if let inst = godotApp?.instance, inst.isStarted() {
            if embedded == nil {
                let displayServerHandle = DisplayServer.shared.handle
                embedded = DisplayServerEmbedded(nativeHandle: displayServerHandle)
            }
            embedded?.resizeWindow(
                size: Vector2i(
                    x: Int32(bounds.size.width * contentScaleFactor),
                    y: Int32(bounds.size.height * contentScaleFactor)
                ),
                id: Int32(DisplayServer.mainWindowId)
            )
        }
        super.layoutSubviews()
    }

    /// Called from updateUIView — idempotent.
    func startGodotInstance() {
        guard !hasStarted else { return }
        guard let inst = godotApp?.instance else { return }
        guard !inst.isStarted() else { hasStarted = true; return }
        hasStarted = true
        let surface = RenderingNativeSurfaceApple.create(
            layer: UInt(bitPattern: Unmanaged.passUnretained(renderingLayer!).toOpaque())
        )
        DisplayServerEmbedded.setNativeSurface(surface)
        let started = inst.start()
        Logger.godot.info("[GodotEmbed] inst.start() returned \(started)")
        displayLink = CADisplayLink(target: self, selector: #selector(iterate))
        displayLink?.add(to: .current, forMode: .default)
    }

    override func didMoveToSuperview() {
        commonInit()
        startGodotInstance()
    }

    /// Destroy the Godot instance when removed from the view hierarchy.
    /// This is the clean stop/restart hook: replacing the VifuGodotApp @State
    /// in ContentView causes SwiftUI to remove this view, which fires this method.
    override func removeFromSuperview() {
        displayLink?.invalidate()
        displayLink = nil
        embedded = nil
        if let inst = godotApp?.instance {
            Logger.godot.info("[GodotEmbed] Destroying Godot instance")
            // Clear the native surface before destroying the instance.
            // This matches the react-native-godot destroy sequence and avoids a dangling
            // Metal layer reference inside libgodot's DisplayServerEmbedded.
            DisplayServerEmbedded.setNativeSurface(nil)
            GodotInstance.destroy(instance: inst)  // calls dlclose internally
            godotApp?.markInstanceDestroyed()       // synchronous — no async Task
        }
        super.removeFromSuperview()
    }

    @objc private func iterate() {
        guard let inst = godotApp?.instance, inst.isStarted() else { return }
        _ = inst.iteration()
    }

    // MARK: Touch forwarding

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        let scale = renderingLayer.contentsScale
        guard godotApp?.instance != nil else { return }
        let winId = Int32(DisplayServer.mainWindowId)
        for touch in touches {
            let id = godotApp!.getTouchId(touch: touch)
            guard id >= 0 else { continue }
            var loc = touch.location(in: self)
            guard renderingLayer.frame.contains(loc) else { continue }
            loc.x -= renderingLayer.frame.origin.x
            loc.y -= renderingLayer.frame.origin.y
            (DisplayServer.shared as! DisplayServerEmbedded).touchPress(
                idx: Int32(id),
                x: Int32(loc.x * scale), y: Int32(loc.y * scale),
                pressed: true, doubleClick: touch.tapCount > 1, window: winId
            )
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        let scale = renderingLayer.contentsScale
        guard godotApp?.instance != nil else { return }
        let winId = Int32(DisplayServer.mainWindowId)
        for touch in touches {
            let id = godotApp!.touchIndex(for: touch)
            guard id >= 0 else { continue }
            var loc = touch.location(in: self)
            guard renderingLayer.frame.contains(loc) else { continue }
            loc.x -= renderingLayer.frame.origin.x
            loc.y -= renderingLayer.frame.origin.y
            var prev = touch.previousLocation(in: self)
            guard renderingLayer.frame.contains(prev) else { continue }
            prev.x -= renderingLayer.frame.origin.x
            prev.y -= renderingLayer.frame.origin.y
            let alt = touch.altitudeAngle
            let azim = touch.azimuthUnitVector(in: self)
            let force = touch.force
            let maxForce = touch.maximumPossibleForce
            (DisplayServer.shared as! DisplayServerEmbedded).touchDrag(
                idx: Int32(id),
                prevX: Int32(prev.x * scale), prevY: Int32(prev.y * scale),
                x: Int32(loc.x * scale), y: Int32(loc.y * scale),
                pressure: maxForce > 0 ? Double(force) / Double(maxForce) : 0,
                tilt: Vector2(
                    x: Float(azim.dx) * Float(cos(alt)),
                    y: Float(azim.dy) * cos(Float(alt))
                ),
                window: winId
            )
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        let scale = renderingLayer.contentsScale
        guard godotApp?.instance != nil else { return }
        let winId = Int32(DisplayServer.mainWindowId)
        for touch in touches {
            let id = godotApp!.touchIndex(for: touch)
            guard id >= 0 else { continue }
            godotApp!.removeTouchId(id: id)
            var loc = touch.location(in: self)
            guard renderingLayer.frame.contains(loc) else { continue }
            loc.x -= renderingLayer.frame.origin.x
            loc.y -= renderingLayer.frame.origin.y
            (DisplayServer.shared as! DisplayServerEmbedded).touchPress(
                idx: Int32(id),
                x: Int32(loc.x * scale), y: Int32(loc.y * scale),
                pressed: false, doubleClick: false, window: winId
            )
        }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard godotApp?.instance != nil else { return }
        let winId = Int32(DisplayServer.mainWindowId)
        for touch in touches {
            let id = godotApp!.touchIndex(for: touch)
            guard id >= 0 else { continue }
            godotApp!.removeTouchId(id: id)
            (DisplayServer.shared as! DisplayServerEmbedded).touchesCanceled(
                idx: Int32(id), window: winId
            )
        }
    }
}

// MARK: - VifuGodotView

/// SwiftUI UIViewRepresentable wrapper for UIVifuGodotView.
/// Pass a VifuGodotApp whose boot() has already been called.
/// Use .id(someUUID) on the parent to force re-creation on restart.
struct VifuGodotView: UIViewRepresentable {
    let godotApp: VifuGodotApp
    var onEdgeSwipeExit: (() -> Void)?
    // Store the view so the same UIView is returned across re-renders.
    private let uiView = UIVifuGodotView()

    func makeUIView(context: Context) -> UIVifuGodotView {
        uiView.contentScaleFactor = UIScreen.main.scale
        uiView.isMultipleTouchEnabled = true
        uiView.godotApp = godotApp
        uiView.onEdgeSwipeExit = onEdgeSwipeExit
        return uiView
    }

    func updateUIView(_ uiView: UIVifuGodotView, context: Context) {
        uiView.startGodotInstance()
    }
}

#elseif os(macOS)
import AppKit
import SwiftUI
@preconcurrency import SwiftGodot
@preconcurrency import SwiftGodotKit
import OSLog

private extension Logger {
    static let godot = Logger(
        subsystem: "dev.vifu.godot-ios-embedding",
        category: "Godot"
    )
}

// macOS: Godot opens its own NSWindow; no embedded CAMetalLayer needed.
// We just create the instance, start it, and tick it on a background task.

@MainActor
final class VifuGodotApp: ObservableObject {
    @Published private(set) var instance: GodotInstance?
    private var iterationTask: Task<Void, Never>?

    func boot(packFile: String) {
        guard instance == nil else {
            Logger.godot.warning("[GodotEmbed] boot() called on already-booted app — ignoring")
            return
        }
        let resourcePath = Bundle.main.resourcePath ?? "."
        let newInstance = GodotInstance.create(args: [
            "--main-pack", "\(resourcePath)/\(packFile).pck",
            "--rendering-driver", "metal",
            "--rendering-method", "mobile",
            "--display-driver", "macos",
        ])
        guard let newInstance else {
            Logger.godot.error("[GodotEmbed] GodotInstance.create() failed for pack: \(packFile)")
            return
        }
        instance = newInstance
        guard !newInstance.isStarted() else { return }
        newInstance.start()
        iterationTask = Task { @MainActor [weak self] in
            while let inst = self?.instance, inst.isStarted() {
                if inst.iteration() { break }
                try? await Task.sleep(nanoseconds: 16_666_667)
            }
        }
        Logger.godot.info("[GodotEmbed] Godot instance started (macOS) for pack: \(packFile)")
    }

    func boot(packPath: String) {
        guard instance == nil else {
            Logger.godot.warning("[GodotEmbed] boot(packPath:) called on already-booted app — ignoring")
            return
        }
        let newInstance = GodotInstance.create(args: [
            "--main-pack", packPath,
            "--rendering-driver", "metal",
            "--rendering-method", "mobile",
            "--display-driver", "macos",
        ])
        guard let newInstance else {
            Logger.godot.error("[GodotEmbed] GodotInstance.create() failed for pack path: \(packPath)")
            return
        }
        instance = newInstance
        guard !newInstance.isStarted() else { return }
        newInstance.start()
        iterationTask = Task { @MainActor [weak self] in
            while let inst = self?.instance, inst.isStarted() {
                if inst.iteration() { break }
                try? await Task.sleep(nanoseconds: 16_666_667)
            }
        }
        Logger.godot.info("[GodotEmbed] Godot instance started (macOS) for pack path: \(packPath)")
    }

    func markInstanceDestroyed() {
        iterationTask?.cancel()
        iterationTask = nil
        instance = nil
    }

    deinit {
        iterationTask?.cancel()
    }

    // Unused on macOS but required for shared code paths.
    let maxTouchCount = 0
    var touches: [AnyObject?] = []
    func getTouchId(touch: AnyObject) -> Int { return -1 }
    func removeTouchId(id: Int) {}
    func touchIndex(for touch: AnyObject) -> Int { return -1 }
}

/// On macOS, Godot renders in its own window — nothing to embed in SwiftUI.
struct VifuGodotView: View {
    let godotApp: VifuGodotApp
    var body: some View { EmptyView() }
}

#endif // os(iOS) / os(macOS)

#endif // canImport(SwiftGodot) && canImport(SwiftGodotKit)
