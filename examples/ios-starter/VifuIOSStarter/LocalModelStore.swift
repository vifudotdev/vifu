import CryptoKit
import Foundation
import Observation

@MainActor
@Observable
final class LocalModelStore {
    nonisolated static let modelName = "qwen2.5-0.5b-instruct-q4_k_m.gguf"
    nonisolated static let expectedByteCount: Int64 = 491_400_032
    nonisolated static let expectedSHA256 = "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db"
    nonisolated static let modelURL = URL(string:
        "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/" +
        "df5bf01389a39c743ab467d734bf501681e041c5/" +
        "qwen2.5-0.5b-instruct-q4_k_m.gguf"
    )!
    nonisolated private static let selectedModelKey = "selectedModelFilename"

    private(set) var modelURL: URL?
    private(set) var progress = 0.0
    private(set) var statusText = "Ready"
    private(set) var isWorking = false
    private(set) var errorMessage: String?

    private let fileManager = FileManager.default

    init() {
        if let selectedName = UserDefaults.standard.string(forKey: Self.selectedModelKey) {
            let selectedURL = Self.modelsDirectory
                .appendingPathComponent(selectedName, isDirectory: false)
            if fileManager.fileExists(atPath: selectedURL.path) {
                modelURL = selectedURL
            } else {
                UserDefaults.standard.removeObject(forKey: Self.selectedModelKey)
            }
        }
        if modelURL == nil {
            let defaultURL = Self.modelsDirectory
                .appendingPathComponent(Self.modelName, isDirectory: false)
            if fileManager.fileExists(atPath: defaultURL.path) {
                UserDefaults.standard.set(Self.modelName, forKey: Self.selectedModelKey)
                modelURL = defaultURL
            }
        }
    }

    func downloadDefaultModel() async {
        guard !isWorking else { return }
        beginWork("Preparing download")
        let delegate = ModelDownloadDelegate { [weak self] progress in
            Task { @MainActor in
                self?.progress = progress
                self?.statusText = "Downloading \(Int(progress * 100))%"
            }
        }

        do {
            let temporaryURL = try await delegate.download(from: Self.modelURL)
            statusText = "Verifying model"
            progress = 1
            let destination = Self.modelsDirectory
                .appendingPathComponent(Self.modelName, isDirectory: false)
            try await Task.detached(priority: .utility) {
                defer { try? FileManager.default.removeItem(at: temporaryURL) }
                try FileManager.default.createDirectory(
                    at: Self.modelsDirectory,
                    withIntermediateDirectories: true
                )
                try Self.verifyDefaultModel(at: temporaryURL)
                try Self.replaceItem(at: destination, with: temporaryURL)
            }.value
            UserDefaults.standard.set(Self.modelName, forKey: Self.selectedModelKey)
            modelURL = destination
            statusText = "Model ready"
            isWorking = false
        } catch {
            fail(error)
        }
    }

    func importModel(_ result: Result<[URL], Error>) async {
        guard !isWorking else { return }
        do {
            let source = try result.get().first
            guard let source else { return }
            beginWork("Importing model")
            guard source.pathExtension.lowercased() == "gguf" else {
                throw ModelStoreError.invalidFileType
            }
            let safeName = source.lastPathComponent.isEmpty
                ? "ImportedModel.gguf"
                : source.lastPathComponent
            let destination = Self.modelsDirectory
                .appendingPathComponent(safeName, isDirectory: false)
            try await Task.detached(priority: .userInitiated) {
                let accessed = source.startAccessingSecurityScopedResource()
                defer {
                    if accessed { source.stopAccessingSecurityScopedResource() }
                }
                try FileManager.default.createDirectory(
                    at: Self.modelsDirectory,
                    withIntermediateDirectories: true
                )
                let staging = Self.modelsDirectory
                    .appendingPathComponent(".\(UUID().uuidString).download", isDirectory: false)
                defer { try? FileManager.default.removeItem(at: staging) }
                try FileManager.default.copyItem(at: source, to: staging)
                try Self.replaceItem(at: destination, with: staging)
            }.value
            UserDefaults.standard.set(safeName, forKey: Self.selectedModelKey)
            modelURL = destination
            progress = 1
            statusText = "Model ready"
            isWorking = false
        } catch {
            fail(error)
        }
    }

    func chooseAnotherModel() {
        UserDefaults.standard.removeObject(forKey: Self.selectedModelKey)
        modelURL = nil
        errorMessage = nil
        progress = 0
        statusText = "Ready"
    }

    private func beginWork(_ status: String) {
        isWorking = true
        progress = 0
        statusText = status
        errorMessage = nil
    }

    private func fail(_ error: Error) {
        isWorking = false
        progress = 0
        statusText = "Setup stopped"
        errorMessage = error.localizedDescription
    }

    nonisolated private static var modelsDirectory: URL {
        let base = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        )[0]
        return base
            .appendingPathComponent("VifuIOSStarter", isDirectory: true)
            .appendingPathComponent("Models", isDirectory: true)
    }

    nonisolated private static func verifyDefaultModel(at url: URL) throws {
        let values = try url.resourceValues(forKeys: [.fileSizeKey])
        guard values.fileSize.map(Int64.init) == expectedByteCount else {
            throw ModelStoreError.sizeMismatch
        }
        guard try sha256(of: url) == expectedSHA256 else {
            throw ModelStoreError.checksumMismatch
        }
    }

    nonisolated private static func sha256(of url: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        var hasher = SHA256()
        while true {
            guard let data = try handle.read(upToCount: 1024 * 1024),
                  !data.isEmpty else {
                break
            }
            hasher.update(data: data)
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    nonisolated private static func replaceItem(at destination: URL, with source: URL) throws {
        let fileManager = FileManager.default
        if fileManager.fileExists(atPath: destination.path) {
            _ = try fileManager.replaceItemAt(destination, withItemAt: source)
        } else {
            try fileManager.moveItem(at: source, to: destination)
        }
    }
}

private enum ModelStoreError: LocalizedError {
    case invalidFileType
    case sizeMismatch
    case checksumMismatch

    var errorDescription: String? {
        switch self {
        case .invalidFileType:
            "Choose a GGUF model file."
        case .sizeMismatch:
            "The downloaded model has an unexpected size."
        case .checksumMismatch:
            "The downloaded model failed verification."
        }
    }
}

private final class ModelDownloadDelegate: NSObject, URLSessionDownloadDelegate, @unchecked Sendable {
    private let progressHandler: @Sendable (Double) -> Void
    private var continuation: CheckedContinuation<URL, Error>?
    private var session: URLSession?

    init(progressHandler: @escaping @Sendable (Double) -> Void) {
        self.progressHandler = progressHandler
    }

    func download(from url: URL) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            let configuration = URLSessionConfiguration.default
            configuration.waitsForConnectivity = true
            let session = URLSession(
                configuration: configuration,
                delegate: self,
                delegateQueue: nil
            )
            self.session = session
            session.downloadTask(with: url).resume()
        }
    }

    func urlSession(
        _ session: URLSession,
        downloadTask: URLSessionDownloadTask,
        didWriteData bytesWritten: Int64,
        totalBytesWritten: Int64,
        totalBytesExpectedToWrite: Int64
    ) {
        guard totalBytesExpectedToWrite > 0 else { return }
        progressHandler(Double(totalBytesWritten) / Double(totalBytesExpectedToWrite))
    }

    func urlSession(
        _ session: URLSession,
        downloadTask: URLSessionDownloadTask,
        didFinishDownloadingTo location: URL
    ) {
        do {
            let preserved = FileManager.default.temporaryDirectory
                .appendingPathComponent(UUID().uuidString, isDirectory: false)
            try FileManager.default.moveItem(at: location, to: preserved)
            finish(.success(preserved))
        } catch {
            finish(.failure(error))
        }
    }

    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        didCompleteWithError error: Error?
    ) {
        if let error {
            finish(.failure(error))
        }
    }

    private func finish(_ result: Result<URL, Error>) {
        guard let continuation else { return }
        self.continuation = nil
        session?.finishTasksAndInvalidate()
        session = nil
        continuation.resume(with: result)
    }
}
