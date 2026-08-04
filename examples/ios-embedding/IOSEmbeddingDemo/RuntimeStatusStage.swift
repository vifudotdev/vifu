import SwiftUI

struct RuntimeStatusStage: View {
    let activity: EmbeddingViewModel.Activity

    var body: some View {
        ZStack {
            LinearGradient(
                colors: [Color.vifuCoral.opacity(0.18), Color.vifuMint.opacity(0.08)],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )

            Circle()
                .fill(Color.white.opacity(0.06))
                .frame(width: 176, height: 176)
                .overlay {
                    Circle()
                        .stroke(accent.opacity(0.45), lineWidth: 2)
                        .padding(8)
                }

            VStack(spacing: 14) {
                Image(systemName: symbol)
                    .font(.system(size: 58, weight: .medium))
                    .foregroundStyle(accent)
                    .accessibilityHidden(true)

                Text(label)
                    .font(.system(size: 14, weight: .semibold, design: .rounded))
                    .foregroundStyle(.secondary)
            }
        }
        .background(Color.appBackground)
        .accessibilityElement(children: .combine)
    }

    private var symbol: String {
        switch activity {
        case .loading:
            "arrow.triangle.2.circlepath"
        case .idle:
            "sparkles"
        case .thinking:
            "ellipsis.bubble.fill"
        case .speaking:
            "waveform"
        case .failed:
            "exclamationmark.triangle.fill"
        }
    }

    private var accent: Color {
        if case .failed = activity {
            return .red
        }
        return activity == .idle ? .vifuMint : .vifuCoral
    }

    private var label: String {
        switch activity {
        case .loading:
            "Loading runtime"
        case .idle:
            "Ready on device"
        case .thinking:
            "Thinking locally"
        case .speaking:
            "Speaking"
        case .failed:
            "Runtime needs attention"
        }
    }
}
