import SwiftUI

struct GatewayControlView: View {
    @Environment(\.dismiss) private var dismiss
    @Bindable var viewModel: EmbeddingViewModel
    @State private var showsScanner = false
    @State private var pairingCode = ""

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 22) {
                VStack(alignment: .leading, spacing: 7) {
                    Label(viewModel.gatewayLabel, systemImage: "point.3.connected.trianglepath.dotted")
                        .font(.headline)
                    Text("Connect this device to a Vifu project to manage its local agents and inspect runtime activity.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }

                if let gatewayError = viewModel.gatewayError {
                    Text(gatewayError)
                        .font(.footnote)
                        .foregroundStyle(.red)
                }

                VStack(alignment: .leading, spacing: 8) {
                    Text("Pairing code")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    TextEditor(text: $pairingCode)
                        .font(.caption.monospaced())
                        .frame(minHeight: 92)
                        .padding(8)
                        .scrollContentBackground(.hidden)
                        .background(Color.white.opacity(0.08))
                        .clipShape(RoundedRectangle(cornerRadius: 8))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button {
                        pair(pairingCode)
                    } label: {
                        Label("Pair copied code", systemImage: "doc.on.clipboard")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(pairingCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }

#if os(iOS)
                Button {
                    showsScanner = true
                } label: {
                    Label("Scan pairing code", systemImage: "qrcode.viewfinder")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle())
                .sheet(isPresented: $showsScanner) {
                    GatewayCodeScanner { code in
                        showsScanner = false
                        pair(code)
                    }
                }
#endif

                Spacer()
            }
            .padding(24)
            .background(Color.appBackground)
            .navigationTitle("Vifu Gateway")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    private func pair(_ code: String) {
        let trimmed = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        pairingCode = ""
        Task { await viewModel.enrollGateway(pairingCode: trimmed) }
    }
}
