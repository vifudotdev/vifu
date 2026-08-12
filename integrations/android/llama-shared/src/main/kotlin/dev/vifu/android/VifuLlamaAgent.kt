package dev.vifu.android

import android.content.Context
import dev.vifu.llama.VifuLlamaConfig as NativeLlamaConfig
import dev.vifu.llama.VifuLlamaData
import dev.vifu.llama.VifuLlamaException
import dev.vifu.llama.VifuLlamaInvocation
import dev.vifu.llama.VifuLlamaProvider
import dev.vifu.llama.VifuLlamaRequest
import dev.vifu.llama.VifuLlamaResponse
import dev.vifu.llama.VifuLlamaStage
import dev.vifu.runtime.VifuInvocationData
import dev.vifu.runtime.VifuInvocationEventKind
import dev.vifu.runtime.VifuInvocationState
import dev.vifu.runtime.VifuProviderInvocation
import dev.vifu.runtime.VifuProviderRequest
import dev.vifu.runtime.VifuProviderResponse
import dev.vifu.runtime.VifuProviderStage
import dev.vifu.runtime.VifuRuntimeException
import dev.vifu.runtime.VifuStreamingAgentProvider
import java.io.Closeable
import java.io.File
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.CancellationException
import java.util.concurrent.ExecutionException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.delay
import org.json.JSONArray
import org.json.JSONObject

data class VifuLlamaConfig(
    val modelPath: String,
    val contextSize: UInt = 2_048u,
    val defaultMaxTokens: UInt = 128u,
    val systemPrompt: String? = null,
) {
    init {
        require(modelPath.isNotBlank()) { "modelPath must not be blank" }
        require(contextSize > 0u) { "contextSize must be positive" }
        require(defaultMaxTokens > 0u) { "defaultMaxTokens must be positive" }
    }
}

enum class VifuBuildProfile {
    ARM_OPTIMIZED,
    BASELINE,
}

/** A lazily attached llama provider backed by the optional llama AAR. */
class VifuLlamaAgent private constructor(
    private val host: VifuAndroidRuntime,
    private val ownsHost: Boolean,
) : Closeable {
    val buildProfile: VifuBuildProfile = VifuArtifactProfile.profile
    val connectionState = host.connectionState

    private val closed = AtomicBoolean(false)
    private val conversationMutex = Mutex()
    private val conversation = mutableListOf<ChatMessage>()
    private val sessionId = "android-${UUID.randomUUID()}"

    fun send(message: String): Flow<String> = flow {
        require(message.isNotBlank()) { "message must not be blank" }
        check(!closed.get()) { "VifuLlamaAgent is closed" }
        conversationMutex.withLock {
            val requestMessages = conversation + ChatMessage("user", message)
            val request = JSONObject().put(
                "messages",
                JSONArray().also { array ->
                    requestMessages.forEach { item ->
                        array.put(JSONObject().put("role", item.role).put("content", item.content))
                    }
                },
            )
            val runtime = host.nativeRuntime()
            val handle = runtime.startInvoke(
                ENDPOINT,
                sessionId,
                VifuInvocationData.Json(request.toString()),
                "{\"source\":\"vifu-android-llama\"}",
            )
            var finalText: String? = null
            try {
                while (finalText == null) {
                    runtime.drainInvocationEvents(handle).forEach { event ->
                        when (event.kind) {
                            VifuInvocationEventKind.OUTPUT_DELTA -> emit(event.data.jsonString())
                            VifuInvocationEventKind.COMPLETED ->
                                finalText = event.data.assistantText()
                            VifuInvocationEventKind.FAILED ->
                                error(event.error ?: "Vifu invocation failed")
                            VifuInvocationEventKind.CANCELLED ->
                                throw CancellationException("Vifu invocation cancelled")
                            VifuInvocationEventKind.STARTED -> Unit
                        }
                    }
                    if (finalText != null) break
                    val poll = runtime.pollInvocation(handle)
                    when (poll.state) {
                        VifuInvocationState.COMPLETED ->
                            finalText = poll.result?.data.assistantText()
                                ?: error("Vifu invocation completed without output")
                        VifuInvocationState.FAILED ->
                            error(poll.error ?: "Vifu invocation failed")
                        VifuInvocationState.CANCELLED ->
                            throw CancellationException("Vifu invocation cancelled")
                        VifuInvocationState.PENDING, VifuInvocationState.RUNNING ->
                            delay(POLL_INTERVAL_MS)
                    }
                }
                conversation += ChatMessage("user", message)
                conversation += ChatMessage("assistant", requireNotNull(finalText))
            } catch (cancelled: CancellationException) {
                runCatching { runtime.cancelInvocation(handle) }
                throw cancelled
            }
        }
    }

    suspend fun resetConversation() = conversationMutex.withLock { conversation.clear() }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        if (ownsHost) host.close() else runCatching { host.unloadProvider(PROVIDER_ID) }
    }

    companion object {
        suspend fun attach(
            runtime: VifuAndroidRuntime,
            model: VifuLlamaConfig,
        ): VifuLlamaAgent = attachInternal(runtime, model, ownsHost = false)

        suspend fun open(
            context: Context,
            model: VifuLlamaConfig,
        ): VifuLlamaAgent = openInternal(context, connection = null, model)

        suspend fun open(
            context: Context,
            connection: VifuConnectionConfig,
            model: VifuLlamaConfig,
        ): VifuLlamaAgent = openInternal(context, connection, model)

        private suspend fun openInternal(
            context: Context,
            connection: VifuConnectionConfig?,
            model: VifuLlamaConfig,
        ): VifuLlamaAgent {
            val modelFile = File(model.modelPath)
            val runtime = VifuAndroidRuntime.open(
                context = context,
                scope = "llama-${sha256(modelFile.canonicalPath).take(16)}",
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
            model: VifuLlamaConfig,
            ownsHost: Boolean,
        ): VifuLlamaAgent = withContext(Dispatchers.IO) {
            require(File(model.modelPath).isFile) {
                "modelPath must point to a readable GGUF file"
            }
            val provider = VifuLlamaProvider.loadWithBackends(
                NativeLlamaConfig(
                    modelPath = model.modelPath,
                    contextSize = model.contextSize,
                    gpuLayers = 0u,
                    defaultMaxTokens = model.defaultMaxTokens,
                ),
                runtime.nativeLibraryDirectory(),
            )
            val bridge = LlamaProviderBridge(provider)
            runtime.installProvider(
                providerId = PROVIDER_ID,
                providerType = "llama",
                provider = bridge,
                resource = bridge,
                agentId = AGENT_ID,
                agentName = "Android local llama",
                capabilities = listOf("chat"),
                agentMetadataJson = model.systemPrompt
                    ?.let {
                        JSONObject().put(
                            "persona",
                            JSONObject().put("instructions", it),
                        ).toString()
                    }
                    ?: "{}",
                endpoint = ENDPOINT,
                endpointCapability = "chat",
                timeoutMs = TIMEOUT_MS,
            )
            VifuLlamaAgent(runtime, ownsHost)
        }

        private fun sha256(value: String): String = MessageDigest.getInstance("SHA-256")
            .digest(value.toByteArray())
            .joinToString("") { "%02x".format(it) }

        private const val PROVIDER_ID = "android-llama"
        private const val AGENT_ID = "android-local-chat"
        private const val ENDPOINT = "chat"
        private const val TIMEOUT_MS = 120_000uL
        private const val POLL_INTERVAL_MS = 16L
    }
}

private class LlamaProviderBridge(
    private val provider: VifuLlamaProvider,
) : VifuStreamingAgentProvider, AutoCloseable {
    // A Core callback arrives on a native Rust thread that JNA attached to the
    // JVM. Entering a second JNA library on that same callback stack makes ART
    // detach the thread too early. A JVM-owned worker removes that nesting.
    private val executor = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "vifu-llama-provider").apply { isDaemon = true }
    }
    private val submissionLock = Any()
    private val closed = AtomicBoolean(false)

    override fun invoke(
        request: VifuProviderRequest,
        invocation: VifuProviderInvocation,
    ): VifuProviderResponse {
        val future = synchronized(submissionLock) {
            if (closed.get()) {
                throw VifuRuntimeException.Runtime("llama provider is closed")
            }
            executor.submit<VifuProviderResponse> { invokeNative(request, invocation) }
        }
        return try {
            future.get()
        } catch (error: InterruptedException) {
            Thread.currentThread().interrupt()
            throw VifuRuntimeException.Runtime("llama provider invocation was interrupted")
        } catch (error: ExecutionException) {
            when (val cause = error.cause) {
                is VifuRuntimeException -> throw cause
                is RuntimeException -> throw cause
                else -> throw VifuRuntimeException.Runtime(
                    cause?.message ?: "llama provider invocation failed",
                )
            }
        }
    }

    private fun invokeNative(
        request: VifuProviderRequest,
        invocation: VifuProviderInvocation,
    ): VifuProviderResponse = try {
        provider.invoke(request.toLlama(), LlamaInvocationBridge(invocation)).toCore()
    } catch (error: VifuLlamaException.InvalidConfig) {
        throw VifuRuntimeException.InvalidConfig(error.message ?: "Invalid llama configuration")
    } catch (error: VifuLlamaException.Runtime) {
        throw VifuRuntimeException.Runtime(error.message ?: "llama invocation failed")
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

private class LlamaInvocationBridge(
    private val invocation: VifuProviderInvocation,
) : VifuLlamaInvocation {
    override fun isCancelled(): Boolean = invocation.isCancelled()

    override fun outputDeltaJson(json: String) =
        invocation.outputDelta(VifuInvocationData.Json(json))

    override fun outputDeltaBinary(bytes: ByteArray) =
        invocation.outputDelta(VifuInvocationData.Binary(bytes))

    override fun activity() = invocation.activity()

    override fun stageStarted(stage: VifuLlamaStage, metadataJson: String) =
        invocation.stageStarted(stage.toCore(), metadataJson)

    override fun stageCompleted(
        stage: VifuLlamaStage,
        elapsedMs: ULong,
        metadataJson: String,
    ) = invocation.stageCompleted(stage.toCore(), elapsedMs, metadataJson)

    override fun stageFailed(
        stage: VifuLlamaStage,
        elapsedMs: ULong,
        error: String,
        metadataJson: String,
    ) = invocation.stageFailed(stage.toCore(), elapsedMs, error, metadataJson)
}

private fun VifuProviderRequest.toLlama() = VifuLlamaRequest(
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
        is VifuInvocationData.Json -> VifuLlamaData.Json(value.json)
        is VifuInvocationData.Binary -> VifuLlamaData.Binary(value.bytes)
    },
    metadataJson = metadataJson,
    stateJson = stateJson,
    stateRevision = stateRevision,
)

private fun VifuLlamaResponse.toCore() = VifuProviderResponse(
    data = when (val value = data) {
        is VifuLlamaData.Json -> VifuInvocationData.Json(value.json)
        is VifuLlamaData.Binary -> VifuInvocationData.Binary(value.bytes)
    },
    metadataJson = metadataJson,
    stateJson = stateJson,
)

private fun VifuLlamaStage.toCore(): VifuProviderStage = when (this) {
    VifuLlamaStage.QUEUE -> VifuProviderStage.QUEUE
    VifuLlamaStage.LOAD -> VifuProviderStage.LOAD
    VifuLlamaStage.TOKENIZE -> VifuProviderStage.TOKENIZE
    VifuLlamaStage.PREFILL -> VifuProviderStage.PREFILL
    VifuLlamaStage.FIRST_TOKEN -> VifuProviderStage.FIRST_TOKEN
    VifuLlamaStage.DECODE -> VifuProviderStage.DECODE
    VifuLlamaStage.VALIDATE -> VifuProviderStage.VALIDATE
}

private data class ChatMessage(val role: String, val content: String)

private fun VifuInvocationData?.jsonString(): String = when (this) {
    is VifuInvocationData.Json -> JSONObject("{\"value\":$json}").getString("value")
    is VifuInvocationData.Binary -> error("Vifu invocation returned binary data")
    null -> ""
}

private fun VifuInvocationData?.assistantText(): String = when (this) {
    is VifuInvocationData.Json -> JSONObject(json).getString("text")
    is VifuInvocationData.Binary -> error("Vifu invocation returned binary data")
    null -> error("Vifu invocation returned no data")
}
