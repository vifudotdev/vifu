import SwiftUI

struct GatewayControlView: View {
    @Environment(\.dismiss) private var dismiss
    @Bindable var viewModel: GodotEmbeddingViewModel
    @State private var showsScanner = false

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
                        Task { await viewModel.enrollGateway(pairingCode: code) }
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
}
