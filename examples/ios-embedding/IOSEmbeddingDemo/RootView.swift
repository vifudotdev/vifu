import SwiftUI

struct RootView: View {
    @Environment(LocalModelStore.self) private var modelStore

    var body: some View {
        Group {
            if let modelURL = modelStore.modelURL {
                EmbeddingView(modelURL: modelURL)
            } else {
                ModelSetupView()
            }
        }
        .background(Color.appBackground.ignoresSafeArea())
    }
}

private struct ModelSetupView: View {
    @Environment(LocalModelStore.self) private var modelStore
    @State private var isImporting = false

    var body: some View {
        @Bindable var modelStore = modelStore

        VStack(alignment: .leading, spacing: 24) {
            Spacer(minLength: 20)

            CharacterMark()
                .frame(width: 72, height: 72)

            VStack(alignment: .leading, spacing: 10) {
                Text("Run Vifu on iOS")
                    .font(.system(size: 34, weight: .bold, design: .rounded))
                Text("An embedded Vifu Runtime powered by a language model that runs on this device.")
                    .font(.system(size: 17, weight: .regular))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            VStack(alignment: .leading, spacing: 14) {
                Label("Local conversations", systemImage: "lock.shield")
                Label("Streaming responses", systemImage: "text.bubble")
                Label("System voice playback", systemImage: "waveform")
            }
            .font(.system(size: 15, weight: .medium))

            if modelStore.isWorking {
                VStack(alignment: .leading, spacing: 8) {
                    ProgressView(value: modelStore.progress)
                        .tint(.vifuCoral)
                    Text(modelStore.statusText)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            if let errorMessage = modelStore.errorMessage {
                Text(errorMessage)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Spacer()

            VStack(spacing: 12) {
                Button {
                    Task { await modelStore.downloadDefaultModel() }
                } label: {
                    Label("Download model (1.2 GB)", systemImage: "arrow.down.circle.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle())
                .disabled(modelStore.isWorking)

                Button {
                    isImporting = true
                } label: {
                    Label("Import a GGUF model", systemImage: "folder")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(SecondaryButtonStyle())
                .disabled(modelStore.isWorking)
            }
        }
        .padding(24)
        .fileImporter(
            isPresented: $isImporting,
            allowedContentTypes: [.data],
            allowsMultipleSelection: false
        ) { result in
            Task { await modelStore.importModel(result) }
        }
    }
}

private struct CharacterMark: View {
    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 18)
                .fill(Color.vifuCoral)
            Image(systemName: "sparkles")
                .font(.system(size: 30, weight: .semibold))
                .foregroundStyle(.black)
        }
    }
}

struct PrimaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 16, weight: .semibold))
            .padding(.vertical, 15)
            .foregroundStyle(.black)
            .background(configuration.isPressed ? Color.vifuCoral.opacity(0.8) : .vifuCoral)
            .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

struct SecondaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 16, weight: .semibold))
            .padding(.vertical, 14)
            .foregroundStyle(.primary)
            .background(configuration.isPressed ? Color.white.opacity(0.14) : Color.white.opacity(0.08))
            .overlay {
                RoundedRectangle(cornerRadius: 8)
                    .stroke(Color.white.opacity(0.14))
            }
            .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

extension Color {
    static let appBackground = Color(red: 0.055, green: 0.06, blue: 0.075)
    static let panelBackground = Color(red: 0.09, green: 0.10, blue: 0.12)
    static let vifuCoral = Color(red: 1.0, green: 0.48, blue: 0.36)
    static let vifuMint = Color(red: 0.38, green: 0.86, blue: 0.70)
}
