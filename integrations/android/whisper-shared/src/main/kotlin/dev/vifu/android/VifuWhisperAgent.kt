package dev.vifu.android

import android.content.Context
import dev.vifu.runtime.VifuInvocationData
import dev.vifu.runtime.VifuInvocationState
import dev.vifu.runtime.VifuProviderInvocation
import dev.vifu.runtime.VifuProviderRequest
import dev.vifu.runtime.VifuProviderResponse
import dev.vifu.runtime.VifuProviderStage
import dev.vifu.runtime.VifuRuntimeException
import dev.vifu.runtime.VifuStreamingAgentProvider
import dev.vifu.whisper.VifuWhisperConfig as NativeWhisperConfig
import dev.vifu.whisper.VifuWhisperData
import dev.vifu.whisper.VifuWhisperException
import dev.vifu.whisper.VifuWhisperInvocation
import dev.vifu.whisper.VifuWhisperProvider
import dev.vifu.whisper.VifuWhisperRequest
import dev.vifu.whisper.VifuWhisperResponse
import dev.vifu.whisper.VifuWhisperStage
import java.io.Closeable
import java.io.File
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.CancellationException
import java.util.concurrent.ExecutionException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext
import org.json.JSONObject

data class VifuWhisperConfig(
    val modelPath: String,
    val language: String? = null,
) {
    init {
        require(modelPath.isNotBlank()) { "modelPath must not be blank" }
    }
}

/** A lazily attached local Whisper transcription provider. */
class VifuWhisperAgent private constructor(
    private val host: VifuAndroidRuntime,
    private val ownsHost: Boolean,
) : Closeable {
    val connectionState = host.connectionState
    private val closed = AtomicBoolean(false)
    private val sessionId = "android-whisper-${UUID.randomUUID()}"

    suspend fun transcribe(wavAudio: ByteArray): String = withContext(Dispatchers.IO) {
        require(wavAudio.isNotEmpty()) { "wavAudio must not be empty" }
        check(!closed.get()) { "VifuWhisperAgent is closed" }
        val runtime = host.nativeRuntime()
        val handle = runtime.startInvoke(
            ENDPOINT,
            sessionId,
            VifuInvocationData.Binary(wavAudio),
            "{\"source\":\"vifu-android-whisper\"}",
        )
        try {
            while (currentCoroutineContext().isActive) {
                val poll = runtime.pollInvocation(handle)
                when (poll.state) {
                    VifuInvocationState.COMPLETED ->
                        return@withContext poll.result?.data.transcriptionText()
                            ?: error("Vifu transcription completed without output")
                    VifuInvocationState.FAILED ->
                        error(poll.error ?: "Vifu transcription failed")
                    VifuInvocationState.CANCELLED ->
                        throw CancellationException("Vifu transcription cancelled")
                    VifuInvocationState.PENDING, VifuInvocationState.RUNNING ->
                        delay(POLL_INTERVAL_MS)
                }
            }
            throw CancellationException("Vifu transcription cancelled")
        } catch (cancelled: CancellationException) {
            runCatching { runtime.cancelInvocation(handle) }
            throw cancelled
        }
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        if (ownsHost) host.close() else runCatching { host.unloadProvider(PROVIDER_ID) }
    }

    companion object {
        suspend fun attach(
            runtime: VifuAndroidRuntime,
            model: VifuWhisperConfig,
        ): VifuWhisperAgent = attachInternal(runtime, model, ownsHost = false)

        suspend fun open(
            context: Context,
            model: VifuWhisperConfig,
        ): VifuWhisperAgent = openInternal(context, connection = null, model)

        suspend fun open(
            context: Context,
            connection: VifuConnectionConfig,
            model: VifuWhisperConfig,
        ): VifuWhisperAgent = openInternal(context, connection, model)

        private suspend fun openInternal(
            context: Context,
            connection: VifuConnectionConfig?,
            model: VifuWhisperConfig,
        ): VifuWhisperAgent {
            val modelFile = File(model.modelPath)
            val runtime = VifuAndroidRuntime.open(
                context = context,
                scope = "whisper-${sha256(modelFile.canonicalPath).take(16)}",
                connection = connection,
            )
            return try {
                attachInternal(runtime, model, ownsHost = true).also {
                    if (connection != null) runtime.startGateway()
                }
            } catch (error: Throwable) {
                runtime.close()
                throw error
            }
        }

        private suspend fun attachInternal(
            runtime: VifuAndroidRuntime,
            model: VifuWhisperConfig,
            ownsHost: Boolean,
        ): VifuWhisperAgent = withContext(Dispatchers.IO) {
            require(File(model.modelPath).isFile) {
                "modelPath must point to a readable Whisper model"
            }
            val provider = VifuWhisperProvider.load(
                NativeWhisperConfig(
                    providerId = PROVIDER_ID,
                    modelPath = model.modelPath,
                    language = model.language,
                ),
            )
            val bridge = WhisperProviderBridge(provider)
            runtime.installProvider(
                providerId = PROVIDER_ID,
                providerType = "local-whisper",
                provider = bridge,
                resource = bridge,
                agentId = AGENT_ID,
                agentName = "Android local Whisper",
                capabilities = listOf("transcription"),
                agentMetadataJson = "{}",
                endpoint = ENDPOINT,
                endpointCapability = "transcription",
                timeoutMs = TIMEOUT_MS,
            )
            VifuWhisperAgent(runtime, ownsHost)
        }

        private fun sha256(value: String): String = MessageDigest.getInstance("SHA-256")
            .digest(value.toByteArray())
            .joinToString("") { "%02x".format(it) }

        private const val PROVIDER_ID = "android-whisper"
        private const val AGENT_ID = "android-local-whisper"
        private const val ENDPOINT = "transcribe"
        private const val TIMEOUT_MS = 120_000uL
        private const val POLL_INTERVAL_MS = 16L
    }
}

private class WhisperProviderBridge(
    private val provider: VifuWhisperProvider,
) : VifuStreamingAgentProvider, AutoCloseable {
    private val executor = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "vifu-whisper-provider").apply { isDaemon = true }
    }
    private val submissionLock = Any()
    private val closed = AtomicBoolean(false)

    override fun invoke(
        request: VifuProviderRequest,
        invocation: VifuProviderInvocation,
    ): VifuProviderResponse {
        val future = synchronized(submissionLock) {
            if (closed.get()) {
                throw VifuRuntimeException.Runtime("Whisper provider is closed")
            }
            executor.submit<VifuProviderResponse> { invokeNative(request, invocation) }
        }
        return try {
            future.get()
        } catch (error: InterruptedException) {
            Thread.currentThread().interrupt()
            throw VifuRuntimeException.Runtime("Whisper provider invocation was interrupted")
        } catch (error: ExecutionException) {
            when (val cause = error.cause) {
                is VifuRuntimeException -> throw cause
                is RuntimeException -> throw cause
                else -> throw VifuRuntimeException.Runtime(
                    cause?.message ?: "Whisper provider invocation failed",
                )
            }
        }
    }

    private fun invokeNative(
        request: VifuProviderRequest,
        invocation: VifuProviderInvocation,
    ): VifuProviderResponse = try {
        provider.invoke(request.toWhisper(), WhisperInvocationBridge(invocation)).toCore()
    } catch (error: VifuWhisperException.InvalidConfig) {
        throw VifuRuntimeException.InvalidConfig(error.message ?: "Invalid Whisper configuration")
    } catch (error: VifuWhisperException.Runtime) {
        throw VifuRuntimeException.Runtime(error.message ?: "Whisper invocation failed")
    } finally {
        invocation.close()
    }

    override fun close() {
        synchronized(submissionLock) {
            if (!closed.compareAndSet(false, true)) return
            executor.execute { provider.close() }
            executor.shutdown()
        }
    }
}

private class WhisperInvocationBridge(
    private val invocation: VifuProviderInvocation,
) : VifuWhisperInvocation {
    override fun isCancelled(): Boolean = invocation.isCancelled()

    override fun outputDeltaJson(json: String) =
        invocation.outputDelta(VifuInvocationData.Json(json))

    override fun outputDeltaBinary(bytes: ByteArray) =
        invocation.outputDelta(VifuInvocationData.Binary(bytes))

    override fun activity() = invocation.activity()

    override fun stageStarted(stage: VifuWhisperStage, metadataJson: String) =
        invocation.stageStarted(stage.toCore(), metadataJson)

    override fun stageCompleted(
        stage: VifuWhisperStage,
        elapsedMs: ULong,
        metadataJson: String,
    ) = invocation.stageCompleted(stage.toCore(), elapsedMs, metadataJson)

    override fun stageFailed(
        stage: VifuWhisperStage,
        elapsedMs: ULong,
        error: String,
        metadataJson: String,
    ) = invocation.stageFailed(stage.toCore(), elapsedMs, error, metadataJson)
}

private fun VifuProviderRequest.toWhisper() = VifuWhisperRequest(
    projectId = projectId,
    endpoint = endpoint,
    sessionId = sessionId,
    providerId = providerId,
    agentId = agentId,
    agentName = agentName,
    agentCapabilities = agentCapabilities,
    agentMetadataJson = agentMetadataJson,
    capability = capability,
    data = when (val value = data) {
        is VifuInvocationData.Json -> VifuWhisperData.Json(value.json)
        is VifuInvocationData.Binary -> VifuWhisperData.Binary(value.bytes)
    },
    metadataJson = metadataJson,
    stateJson = stateJson,
    stateRevision = stateRevision,
)

private fun VifuWhisperResponse.toCore() = VifuProviderResponse(
    data = when (val value = data) {
        is VifuWhisperData.Json -> VifuInvocationData.Json(value.json)
        is VifuWhisperData.Binary -> VifuInvocationData.Binary(value.bytes)
    },
    metadataJson = metadataJson,
    stateJson = stateJson,
)

private fun VifuWhisperStage.toCore(): VifuProviderStage = when (this) {
    VifuWhisperStage.QUEUE -> VifuProviderStage.QUEUE
    VifuWhisperStage.LOAD -> VifuProviderStage.LOAD
    VifuWhisperStage.TOKENIZE -> VifuProviderStage.TOKENIZE
    VifuWhisperStage.PREFILL -> VifuProviderStage.PREFILL
    VifuWhisperStage.FIRST_TOKEN -> VifuProviderStage.FIRST_TOKEN
    VifuWhisperStage.DECODE -> VifuProviderStage.DECODE
    VifuWhisperStage.VALIDATE -> VifuProviderStage.VALIDATE
}

private fun VifuInvocationData?.transcriptionText(): String = when (this) {
    is VifuInvocationData.Json -> JSONObject(json).getString("text")
    is VifuInvocationData.Binary -> error("Vifu transcription returned binary data")
    null -> error("Vifu transcription returned no data")
}
