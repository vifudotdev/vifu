package dev.vifu.android

import android.content.Context
import dev.vifu.runtime.VifuEmbeddedGateway
import dev.vifu.runtime.VifuEmbeddedGatewayConfig
import dev.vifu.runtime.VifuEmbeddedGatewayState as RawGatewayState
import dev.vifu.runtime.VifuEmbeddedRuntime
import dev.vifu.runtime.VifuStreamingAgentProvider
import dev.vifu.runtime.generateVifuGatewayIdentity
import java.io.Closeable
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Core Vifu runtime. Provider modules are loaded only when attached to this object. */
class VifuAndroidRuntime private constructor(
    private val runtime: VifuEmbeddedRuntime,
    private val databasePath: String,
    private val nativeLibraryDirectory: String,
    private val gateway: VifuEmbeddedGateway?,
    private val connection: VifuConnectionConfig?,
    private val store: VifuCredentialStore?,
    private val identity: VifuStoredIdentity?,
) : Closeable {
    private data class ProviderRegistration(
        val agentId: String,
        val endpoint: String,
        val resource: AutoCloseable,
    )

    private val lock = Any()
    private val closed = AtomicBoolean(false)
    private val providers = linkedMapOf<String, ProviderRegistration>()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val mutableConnectionState = MutableStateFlow<VifuConnectionState>(VifuConnectionState.Stopped)
    val connectionState: StateFlow<VifuConnectionState> = mutableConnectionState.asStateFlow()
    private var gatewayStarted = false
    @Volatile
    private var lastDeviceToken: String? = identity?.deviceToken
    private val statusJob: Job? = gateway?.let { scope.launch { monitorGateway(it) } }

    /** Starts monitoring after the desired provider modules have been attached. */
    suspend fun startGateway() = withContext(Dispatchers.IO) {
        synchronized(lock) {
            checkOpen()
            startGatewayLocked()
        }
    }

    @JvmSynthetic
    fun installProvider(
        providerId: String,
        providerType: String,
        provider: VifuStreamingAgentProvider,
        resource: AutoCloseable,
        agentId: String,
        agentName: String,
        capabilities: List<String>,
        agentMetadataJson: String,
        endpoint: String,
        endpointCapability: String,
        timeoutMs: ULong,
    ) {
        synchronized(lock) {
            checkOpen()
            unloadProviderLocked(providerId, refreshGateway = false)
            try {
                runtime.registerStreamingProvider(providerId, providerType, provider)
                runtime.registerAgent(agentId, agentName, providerId, capabilities, agentMetadataJson)
                runtime.registerEndpoint(endpoint, agentId, endpointCapability, timeoutMs)
                providers[providerId] = ProviderRegistration(agentId, endpoint, resource)
                refreshGatewayLocked()
            } catch (error: Throwable) {
                runCatching { runtime.unregisterEndpoint(endpoint) }
                runCatching { runtime.unregisterAgent(agentId) }
                runCatching { runtime.unregisterProvider(providerId) }
                runCatching { resource.close() }
                throw error
            }
        }
    }

    /** Unregisters the provider and releases its model memory. */
    fun unloadProvider(providerId: String): Boolean = synchronized(lock) {
        checkOpen()
        unloadProviderLocked(providerId, refreshGateway = true)
    }

    @JvmSynthetic
    fun nativeRuntime(): VifuEmbeddedRuntime {
        checkOpen()
        return runtime
    }

    /** Native database path for hosts that manage a custom Gateway bridge. */
    @JvmSynthetic
    fun nativeDatabasePath(): String {
        checkOpen()
        return databasePath
    }

    @JvmSynthetic
    fun nativeLibraryDirectory(): String {
        checkOpen()
        return nativeLibraryDirectory
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        statusJob?.cancel()
        scope.cancel()
        synchronized(lock) {
            gatewayStarted = false
            gateway?.let {
                runCatching { it.stop() }
                it.close()
            }
            providers.keys.toList().forEach { unloadProviderLocked(it, refreshGateway = false) }
            runtime.close()
        }
        mutableConnectionState.value = VifuConnectionState.Stopped
    }

    private fun unloadProviderLocked(providerId: String, refreshGateway: Boolean): Boolean {
        val registration = providers.remove(providerId) ?: return false
        runCatching { runtime.unregisterEndpoint(registration.endpoint) }
        runCatching { runtime.unregisterAgent(registration.agentId) }
        val removed = runCatching { runtime.unregisterProvider(providerId) }.getOrDefault(false)
        runCatching { registration.resource.close() }
        if (refreshGateway) refreshGatewayLocked()
        return removed
    }

    private fun refreshGatewayLocked() {
        if (!gatewayStarted) return
        gateway?.stop()
        gatewayStarted = false
        startGatewayLocked()
    }

    private fun startGatewayLocked() {
        val currentGateway = gateway ?: return
        val currentIdentity = identity ?: error("Vifu Gateway identity is unavailable")
        val currentConnection = connection ?: error("Vifu Gateway configuration is unavailable")
        currentGateway.startWithMonitorIo(
            currentIdentity.privateKey,
            lastDeviceToken,
            currentConnection.appId,
            currentConnection.captureTraceContent,
        )
        gatewayStarted = true
        mutableConnectionState.value = VifuConnectionState.Connecting
    }

    private suspend fun monitorGateway(gateway: VifuEmbeddedGateway) {
        while (scope.isActive) {
            runCatching { gateway.status() }
                .onSuccess { status ->
                    status.authorization?.deviceToken
                        ?.takeIf { it != lastDeviceToken }
                        ?.let {
                            val credentialStore = requireNotNull(store)
                            val storedIdentity = requireNotNull(identity)
                            credentialStore.save(storedIdentity.copy(deviceToken = it))
                            lastDeviceToken = it
                        }
                    mutableConnectionState.value = when (status.state) {
                        RawGatewayState.STOPPED -> VifuConnectionState.Stopped
                        RawGatewayState.CONNECTING -> VifuConnectionState.Connecting
                        RawGatewayState.CONNECTED -> VifuConnectionState.Connected
                        RawGatewayState.RECONNECTING -> VifuConnectionState.Reconnecting
                        RawGatewayState.AUTHORIZATION_REQUIRED ->
                            VifuConnectionState.AuthorizationRequired
                        RawGatewayState.DEGRADED -> VifuConnectionState.Degraded(status.lastError)
                        RawGatewayState.FAILED -> VifuConnectionState.Failed(status.lastError)
                    }
                }
                .onFailure { mutableConnectionState.value = VifuConnectionState.Failed(it.message) }
            delay(GATEWAY_POLL_INTERVAL_MS)
        }
    }

    private fun checkOpen() = check(!closed.get()) { "VifuAndroidRuntime is closed" }

    companion object {
        suspend fun open(
            context: Context,
            projectId: String = "android-app",
            scope: String = "default",
            connection: VifuConnectionConfig? = null,
        ): VifuAndroidRuntime = withContext(Dispatchers.IO) {
            require(projectId.isNotBlank()) { "projectId must not be blank" }
            require(scope.isNotBlank()) { "scope must not be blank" }
            val appContext = context.applicationContext
            val scopeKey = listOf(projectId, scope, connection?.serverUrl, connection?.appId)
                .joinToString("\u0000") { it.orEmpty() }
            val scopeId = sha256(scopeKey).take(24)
            val storage = File(appContext.noBackupFilesDir, "vifu/$scopeId").apply {
                check(exists() || mkdirs()) { "Vifu storage is unavailable" }
            }
            val databasePath = File(storage, "runtime.sqlite").absolutePath
            val runtime = VifuEmbeddedRuntime.open(projectId, databasePath)
            try {
                if (connection == null) {
                    VifuAndroidRuntime(
                        runtime,
                        databasePath,
                        appContext.applicationInfo.nativeLibraryDir,
                        null,
                        null,
                        null,
                        null,
                    )
                } else {
                    val credentialStore = VifuCredentialStore(appContext, scopeId)
                    val identity = credentialStore.load() ?: VifuStoredIdentity(
                        privateKey = generateVifuGatewayIdentity().privateKey,
                        deviceToken = null,
                    ).also(credentialStore::save)
                    val gateway = VifuEmbeddedGateway(
                        runtime,
                        VifuEmbeddedGatewayConfig(
                            serverUrl = connection.serverUrl,
                            runtimeDatabasePath = databasePath,
                            serverCertificateDer = connection.serverCertificateDer,
                        ),
                    )
                    VifuAndroidRuntime(
                        runtime,
                        databasePath,
                        appContext.applicationInfo.nativeLibraryDir,
                        gateway,
                        connection,
                        credentialStore,
                        identity,
                    )
                }
            } catch (error: Throwable) {
                runtime.close()
                throw error
            }
        }

        private fun sha256(value: String): String = MessageDigest.getInstance("SHA-256")
            .digest(value.toByteArray())
            .joinToString("") { "%02x".format(it) }

        private const val GATEWAY_POLL_INTERVAL_MS = 500L
    }
}
