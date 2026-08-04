import SwiftUI

struct EmbeddingView: View {
    @Environment(LocalModelStore.self) private var modelStore
    @State private var viewModel: EmbeddingViewModel
    @State private var showsGateway = false

    init(modelURL: URL) {
        _viewModel = State(initialValue: EmbeddingViewModel(modelURL: modelURL))
    }

    var body: some View {
        @Bindable var viewModel = viewModel

        ZStack(alignment: .bottom) {
            VStack(spacing: 0) {
                EmbeddingHeader(
                    activity: viewModel.activityLabel,
                    onModelTap: modelStore.chooseAnotherModel,
                    onGatewayTap: { showsGateway = true }
                )
                RuntimeStatusStage(activity: viewModel.activity)
                    .frame(maxWidth: .infinity)
                    .frame(height: 270)

                ConversationView(messages: viewModel.messages)
            }

            MessageComposer(
                draft: $viewModel.draft,
                canSend: viewModel.canSend,
                isSpeaking: viewModel.activity == .speaking,
                onSend: { Task { await viewModel.send() } },
                onStop: viewModel.stopSpeaking
            )
        }
        .safeAreaInset(edge: .bottom) {
            RuntimeMetrics(
                firstTokenLatency: viewModel.firstTokenLatency,
                tokensPerSecond: viewModel.tokensPerSecond
            )
        }
        .task { await viewModel.start() }
        .sheet(isPresented: $showsGateway) {
            GatewayControlView(viewModel: viewModel)
        }
        .alert(
            "Local runtime error",
            isPresented: Binding(
                get: {
                    if case .failed = viewModel.activity { true } else { false }
                },
                set: { _ in }
            )
        ) {
            Button("Dismiss") {}
        } message: {
            if case let .failed(message) = viewModel.activity {
                Text(message)
            }
        }
    }
}

private struct EmbeddingHeader: View {
    let activity: String
    let onModelTap: () -> Void
    let onGatewayTap: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Vifu iOS Embedding")
                    .font(.system(size: 20, weight: .bold, design: .rounded))
                HStack(spacing: 6) {
                    Circle()
                        .fill(Color.vifuMint)
                        .frame(width: 7, height: 7)
                    Text(activity)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            Button(action: onGatewayTap) {
                Image(systemName: "link")
                    .font(.system(size: 17, weight: .semibold))
                    .frame(width: 40, height: 40)
            }
            .buttonStyle(.plain)
            .background(Color.white.opacity(0.08))
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .accessibilityLabel("Vifu Gateway")
            Button(action: onModelTap) {
                Image(systemName: "cpu")
                    .font(.system(size: 17, weight: .semibold))
                    .frame(width: 40, height: 40)
            }
            .buttonStyle(.plain)
            .background(Color.white.opacity(0.08))
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .accessibilityLabel("Change local model")
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 12)
    }
}

private struct ConversationView: View {
    let messages: [ChatMessage]

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 12) {
                    ForEach(messages) { message in
                        MessageBubble(message: message)
                            .id(message.id)
                    }
                    Color.clear.frame(height: 100)
                }
                .padding(.horizontal, 16)
                .padding(.top, 14)
            }
            .onChange(of: messages.count) {
                if let last = messages.last {
                    withAnimation { proxy.scrollTo(last.id, anchor: .bottom) }
                }
            }
        }
        .background(Color.panelBackground)
        .clipShape(UnevenRoundedRectangle(topLeadingRadius: 18, topTrailingRadius: 18))
    }
}

private struct MessageBubble: View {
    let message: ChatMessage

    var body: some View {
        HStack {
            if message.role == .user { Spacer(minLength: 48) }
            Text(message.text.isEmpty ? " " : message.text)
                .font(.system(size: 16))
                .foregroundStyle(message.role == .user ? .black : .primary)
                .padding(.horizontal, 14)
                .padding(.vertical, 11)
                .background(message.role == .user ? Color.vifuCoral : Color.white.opacity(0.08))
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .overlay(alignment: .leading) {
                    if message.text.isEmpty {
                        ProgressView()
                            .tint(.secondary)
                            .padding(.leading, 14)
                    }
                }
            if message.role == .assistant { Spacer(minLength: 48) }
        }
    }
}

private struct MessageComposer: View {
    @Binding var draft: String
    let canSend: Bool
    let isSpeaking: Bool
    let onSend: () -> Void
    let onStop: () -> Void

    var body: some View {
        HStack(alignment: .bottom, spacing: 10) {
            TextField("Message the embedded agent", text: $draft, axis: .vertical)
                .lineLimit(1...4)
                .textFieldStyle(.plain)
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .background(Color(red: 0.14, green: 0.15, blue: 0.18))
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .submitLabel(.send)
                .onSubmit {
                    if canSend { onSend() }
                }

            Button(action: isSpeaking ? onStop : onSend) {
                Image(systemName: isSpeaking ? "stop.fill" : "arrow.up")
                    .font(.system(size: 17, weight: .bold))
                    .foregroundStyle(.black)
                    .frame(width: 44, height: 44)
                    .background(canSend || isSpeaking ? Color.vifuCoral : Color.gray)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            }
            .disabled(!canSend && !isSpeaking)
            .accessibilityLabel(isSpeaking ? "Stop speaking" : "Send message")
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 46)
    }
}

private struct RuntimeMetrics: View {
    let firstTokenLatency: TimeInterval?
    let tokensPerSecond: Double?

    var body: some View {
        HStack(spacing: 14) {
            Label("On device", systemImage: "iphone")
            if let firstTokenLatency {
                Text("TTFT \(firstTokenLatency.formatted(.number.precision(.fractionLength(2))))s")
            }
            if let tokensPerSecond {
                Text("\(tokensPerSecond.formatted(.number.precision(.fractionLength(1)))) tok/s")
            }
            Spacer()
        }
        .font(.caption2.monospaced())
        .foregroundStyle(.secondary)
        .padding(.horizontal, 18)
        .frame(height: 34)
        .background(Color.appBackground)
    }
}
