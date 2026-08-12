import SwiftUI

@main
struct VifuGodotIOSStarterApp: App {
    @State private var modelStore = LocalModelStore()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(modelStore)
                .preferredColorScheme(.dark)
        }
    }
}
