package com.example.llama

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.text.InputType
import android.util.Base64
import android.util.Log
import android.widget.EditText
import android.widget.TextView
import android.widget.Toast
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import dev.vifu.android.VifuConnectionConfig
import dev.vifu.android.VifuConnectionState
import dev.vifu.android.VifuLlamaAgent
import dev.vifu.android.VifuLlamaConfig
import com.google.android.material.floatingactionbutton.FloatingActionButton
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.onCompletion
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.io.FileOutputStream
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import java.util.UUID

class MainActivity : AppCompatActivity() {

    // Android views
    private lateinit var ggufTv: TextView
    private lateinit var vifuStatusTv: TextView
    private lateinit var messagesRv: RecyclerView
    private lateinit var userInputEt: EditText
    private lateinit var userActionFab: FloatingActionButton

    private var vifuAgent: VifuLlamaAgent? = null
    private var gatewayStatusJob: Job? = null
    private var activeModelFile: File? = null
    private var pendingPairingCode: String? = null
    private var isRestoringModel = false
    private val vifuConnectionMutex = Mutex()

    // Conversation states
    private var isModelReady = false
    private val messages = mutableListOf<Message>()
    private val lastAssistantMsg = StringBuilder()
    private val messageAdapter = MessageAdapter(messages)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContentView(R.layout.activity_main)

        // Find views
        ggufTv = findViewById(R.id.gguf)
        vifuStatusTv = findViewById(R.id.vifu_status)
        messagesRv = findViewById(R.id.messages)
        messagesRv.layoutManager = LinearLayoutManager(this)
        messagesRv.adapter = messageAdapter
        userInputEt = findViewById(R.id.user_input)
        userActionFab = findViewById(R.id.fab)
        vifuStatusTv.setOnClickListener { showGatewayDialog() }

        // Upon CTA button tapped
        userActionFab.setOnClickListener {
            if (isModelReady) {
                // If model is ready, validate input and send to engine
                handleUserInput()
            } else {
                showModelSetupDialog()
            }
        }

        ensureModelsDirectory()
            .listFiles { file -> file.isFile && file.extension.equals("gguf", ignoreCase = true) }
            ?.maxByOrNull(File::lastModified)
            ?.let(::loadModelFile)
        handlePairingIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handlePairingIntent(intent)
    }

    private fun handlePairingIntent(intent: Intent) {
        if (intent.action != Intent.ACTION_VIEW) return
        val code = intent.data?.toString() ?: return
        pairGateway(code)
    }

    private val getContent = registerForActivityResult(
        ActivityResultContracts.OpenDocument()
    ) { uri ->
        Log.i(TAG, "Model file selected")
        uri?.let { handleSelectedModel(it) }
    }

    private fun showModelSetupDialog() {
        AlertDialog.Builder(this)
            .setTitle("Set up a local model")
            .setMessage("Download the verified Starter model or import a GGUF already on this phone.")
            .setPositiveButton("Download 469 MiB") { _, _ -> downloadDefaultModel() }
            .setNegativeButton("Import GGUF") { _, _ -> getContent.launch(arrayOf("*/*")) }
            .setNeutralButton("Cancel", null)
            .show()
    }

    private fun downloadDefaultModel() {
        userActionFab.isEnabled = false
        userInputEt.hint = "Preparing model download..."
        ggufTv.text = "Downloading the verified Starter model"
        lifecycleScope.launch(Dispatchers.IO) {
            val destination = File(ensureModelsDirectory(), DEFAULT_MODEL_NAME)
            val partial = File(destination.parentFile, "${destination.name}.downloading")
            var connection: HttpURLConnection? = null
            try {
                connection = URL(DEFAULT_MODEL_URL).openConnection() as HttpURLConnection
                connection.connectTimeout = 30_000
                connection.readTimeout = 60_000
                connection.instanceFollowRedirects = true
                connection.setRequestProperty("User-Agent", "Vifu-Android-Starter/${BuildConfig.VERSION_NAME}")
                connection.connect()
                check(connection.responseCode in 200..299) {
                    "Model download returned HTTP ${connection.responseCode}"
                }
                var downloaded = 0L
                var lastPercent = -1
                connection.inputStream.use { input ->
                    FileOutputStream(partial, false).use { output ->
                        val buffer = ByteArray(MODEL_BUFFER_SIZE)
                        while (true) {
                            val count = input.read(buffer)
                            if (count < 0) break
                            output.write(buffer, 0, count)
                            downloaded += count
                            val percent = ((downloaded * 100) / DEFAULT_MODEL_SIZE).toInt().coerceAtMost(100)
                            if (percent != lastPercent) {
                                lastPercent = percent
                                withContext(Dispatchers.Main) {
                                    userInputEt.hint = "Downloading model: $percent%"
                                }
                            }
                        }
                    }
                }
                check(downloaded == DEFAULT_MODEL_SIZE) { "The downloaded model has an unexpected size" }
                check(sha256(partial) == DEFAULT_MODEL_SHA256) {
                    "The downloaded model failed verification"
                }
                Files.move(
                    partial.toPath(),
                    destination.toPath(),
                    StandardCopyOption.ATOMIC_MOVE,
                    StandardCopyOption.REPLACE_EXISTING,
                )
                finishLoadingModel(destination, DEFAULT_MODEL_NAME)
            } catch (error: Throwable) {
                partial.delete()
                showModelLoadError(error)
            } finally {
                connection?.disconnect()
            }
        }
    }

    /**
     * Handles the file Uri from [getContent] result
     */
    private fun handleSelectedModel(uri: Uri) {
        // Update UI states
        userActionFab.isEnabled = false
        userInputEt.hint = "Parsing GGUF..."
        ggufTv.text = "Parsing metadata from selected file \n$uri"

        lifecycleScope.launch(Dispatchers.IO) {
            try {
                val modelName = uri.lastPathSegment
                    ?.substringAfterLast('/')
                    ?.takeIf { it.endsWith(FILE_EXTENSION_GGUF, ignoreCase = true) }
                    ?: "model-${System.currentTimeMillis()}$FILE_EXTENSION_GGUF"
                val modelFile = contentResolver.openInputStream(uri)?.use { input ->
                    ensureModelFile(modelName, input)
                } ?: error("The selected model could not be opened")
                finishLoadingModel(modelFile, modelName)
            } catch (error: Throwable) {
                showModelLoadError(error)
            }
        }
    }

    private fun loadModelFile(modelFile: File) {
        isRestoringModel = true
        userActionFab.isEnabled = false
        userInputEt.hint = "Loading saved GGUF..."
        ggufTv.text = "Loading saved model: ${modelFile.name}"
        lifecycleScope.launch(Dispatchers.IO) {
            try {
                finishLoadingModel(modelFile, modelFile.name)
            } catch (error: Throwable) {
                showModelLoadError(error)
            }
        }
    }

    private suspend fun finishLoadingModel(modelFile: File, modelName: String) {
        activeModelFile = modelFile
        connectVifu(modelFile)
        withContext(Dispatchers.Main) {
            isRestoringModel = false
            ggufTv.text = "Model: $modelName"
            isModelReady = true
            userInputEt.hint = "Type and send a message!"
            userInputEt.isEnabled = true
            userActionFab.setImageResource(R.drawable.outline_send_24)
            userActionFab.isEnabled = true
        }
    }

    private suspend fun showModelLoadError(error: Throwable) {
        Log.e(TAG, "Could not load GGUF", error)
        val detail = error.message ?: "Unknown model load error"
        withContext(Dispatchers.Main) {
            isRestoringModel = false
            ggufTv.text = "Could not load this GGUF:\n$detail"
            isModelReady = false
            userInputEt.hint = "Choose another GGUF model."
            userInputEt.isEnabled = false
            userActionFab.setImageResource(R.drawable.outline_folder_open_24)
            userActionFab.isEnabled = true
            Toast.makeText(
                this@MainActivity,
                modelLoadGuidance(detail, BuildConfig.VIFU_BACKEND),
                Toast.LENGTH_LONG,
            ).show()
        }
    }

    /**
     * Prepare the model file within app's private storage
     */
    private suspend fun ensureModelFile(modelName: String, input: InputStream) =
        withContext(Dispatchers.IO) {
            val destination = File(ensureModelsDirectory(), modelName)
            val partial = File(destination.parentFile, "${destination.name}.importing")
            Log.i(TAG, "Copying selected model into private storage")
            withContext(Dispatchers.Main) {
                userInputEt.hint = "Copying file..."
            }
            try {
                FileOutputStream(partial, false).use { output -> input.copyTo(output) }
                check(partial.length() > 0L) { "The selected GGUF is empty" }
                Files.move(
                    partial.toPath(),
                    destination.toPath(),
                    StandardCopyOption.ATOMIC_MOVE,
                    StandardCopyOption.REPLACE_EXISTING,
                )
            } catch (error: Throwable) {
                partial.delete()
                throw error
            }
            destination.also {
                Log.i(TAG, "Model copy finished")
            }
        }

    /**
     * Load the model file from the app private storage
     */
    private suspend fun connectVifu(
        modelFile: File,
        allowGateway: Boolean = true,
    ) = withContext(Dispatchers.Default) {
        vifuConnectionMutex.withLock {
            withContext(Dispatchers.Main) { userInputEt.hint = "Loading model..." }
            gatewayStatusJob?.cancel()
            gatewayStatusJob = null
            vifuAgent?.close()
            vifuAgent = null
            val model = VifuLlamaConfig(modelPath = modelFile.absolutePath)
            val pairingCode = pendingPairingCode
            vifuAgent = if (allowGateway) {
                try {
                    VifuLlamaAgent.connect(
                        context = applicationContext,
                        model = model,
                        pairingCode = pairingCode,
                        defaultConnection = buildTimeConnection(),
                        captureTraceContent = true,
                    ).also { agent ->
                        gatewayStatusJob = lifecycleScope.launch {
                            launch {
                                agent.connectionState.collect { state ->
                                    if (
                                        state == VifuConnectionState.Connected &&
                                        pairingCode != null &&
                                        pendingPairingCode == pairingCode
                                    ) {
                                        pendingPairingCode = null
                                    }
                                    vifuStatusTv.text = if (agent.hasGatewayConnection) {
                                        "Vifu: ${state.label()} · tap to pair"
                                    } else {
                                        "Vifu: local · tap to pair"
                                    }
                                }
                            }
                            launch {
                                var previousError: String? = null
                                agent.connectionError.collect { error ->
                                    if (!error.isNullOrBlank() && error != previousError) {
                                        Log.w(TAG, "Vifu Gateway: $error")
                                    }
                                    previousError = error
                                }
                            }
                        }
                    }
                } catch (error: Throwable) {
                    Log.e(TAG, "Vifu Gateway setup failed; keeping the local agent available", error)
                    withContext(Dispatchers.Main) {
                        vifuStatusTv.text = "Vifu: pairing failed · local · tap to retry"
                    }
                    VifuLlamaAgent.open(applicationContext, model)
                }
            } else {
                VifuLlamaAgent.clearConnection(applicationContext)
                VifuLlamaAgent.open(applicationContext, model).also {
                    withContext(Dispatchers.Main) {
                        vifuStatusTv.text = "Vifu: local · tap to pair"
                    }
                }
            }
        }
    }

    private fun showGatewayDialog() {
        val input = EditText(this).apply {
            hint = "vifu://gateway/enroll?..."
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            setSingleLine(true)
            setHorizontallyScrolling(true)
        }
        AlertDialog.Builder(this)
            .setTitle("Pair with Vifu")
            .setMessage(
                "In the Vifu Dashboard, choose Pair gateway and copy the pairing code. " +
                    "Pairing shares bounded chat input and output with your Vifu Server " +
                    "for tracing.",
            )
            .setView(input)
            .setPositiveButton("Pair") { _, _ -> pairGateway(input.text.toString()) }
            .setNegativeButton("Cancel", null)
            .setNeutralButton("Use local only") { _, _ -> disconnectGateway() }
            .show()
    }

    private fun pairGateway(code: String) {
        val pairingCode = code.trim()
        if (pairingCode.isEmpty()) {
            Toast.makeText(this, "Paste a Vifu pairing code.", Toast.LENGTH_LONG).show()
            return
        }
        pendingPairingCode = pairingCode
        if (isRestoringModel) {
            vifuStatusTv.text = "Vifu: pairing after saved model loads"
            return
        }
        val modelFile = activeModelFile
        if (modelFile == null) {
            pendingPairingCode = null
            vifuStatusTv.text = "Vifu: set up the model, then pair"
            Toast.makeText(
                this,
                "Set up the local model, then scan a fresh Vifu pairing code.",
                Toast.LENGTH_LONG,
            ).show()
            return
        }
        lifecycleScope.launch(Dispatchers.Default) {
            runCatching { connectVifu(modelFile) }
                .onFailure { error ->
                    Log.e(TAG, "Could not pair Vifu Gateway", error)
                    withContext(Dispatchers.Main) {
                        vifuStatusTv.text = "Vifu: pairing failed · tap to retry"
                        Toast.makeText(
                            this@MainActivity,
                            error.message ?: "Vifu pairing failed.",
                            Toast.LENGTH_LONG,
                        ).show()
                    }
                }
        }
    }

    private fun disconnectGateway() {
        VifuLlamaAgent.clearConnection(applicationContext)
        pendingPairingCode = null
        val modelFile = activeModelFile
        if (modelFile == null) {
            vifuStatusTv.text = "Vifu: local · tap to pair"
            return
        }
        lifecycleScope.launch(Dispatchers.Default) { connectVifu(modelFile, allowGateway = false) }
    }

    private fun buildTimeConnection(): VifuConnectionConfig? {
        if (BuildConfig.VIFU_SERVER_URL.isBlank() || BuildConfig.VIFU_APP_ID.isBlank()) return null
        return VifuConnectionConfig(
            serverUrl = BuildConfig.VIFU_SERVER_URL,
            appId = BuildConfig.VIFU_APP_ID,
            gatewayAttributes = mapOf("buildProfile" to BuildConfig.VIFU_BACKEND),
            serverCertificateDer = BuildConfig.VIFU_SERVER_CERTIFICATE_DER_BASE64
                .takeIf(String::isNotBlank)
                ?.let { Base64.decode(it, Base64.DEFAULT) },
        )
    }

    private fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(MODEL_BUFFER_SIZE)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                digest.update(buffer, 0, count)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    /**
     * Validate and send the user message into Vifu's local llama.cpp agent.
     */
    private fun handleUserInput() {
        userInputEt.text.toString().trim().also { userMessage ->
            if (userMessage.isEmpty()) {
                Toast.makeText(this, "Input message is empty!", Toast.LENGTH_SHORT).show()
            } else {
                userInputEt.text = null
                userActionFab.isEnabled = false

                // Update message states
                messages.add(Message(UUID.randomUUID().toString(), userMessage, true))
                lastAssistantMsg.clear()
                messages.add(Message(UUID.randomUUID().toString(), lastAssistantMsg.toString(), false))

                lifecycleScope.launch(Dispatchers.Default) {
                    try {
                        requireNotNull(vifuAgent) { "The local agent is not ready" }
                            .send(userMessage)
                            .onCompletion { error ->
                                error?.let { Log.w(TAG, "Local chat turn stopped", it) }
                            }.collect { token ->
                                val messageCount = messages.size
                                check(messageCount > 0 && !messages[messageCount - 1].isUser)

                                messages.removeAt(messageCount - 1).copy(
                                    content = lastAssistantMsg.append(token).toString()
                                ).let { messages.add(it) }

                                withContext(Dispatchers.Main) {
                                    messageAdapter.notifyItemChanged(messages.size - 1)
                                }
                            }
                    } catch (error: Throwable) {
                        Log.e(TAG, "Local chat turn failed", error)
                        withContext(Dispatchers.Main) {
                            Toast.makeText(
                                this@MainActivity,
                                error.message ?: "The local chat turn failed.",
                                Toast.LENGTH_LONG,
                            ).show()
                        }
                    } finally {
                        withContext(Dispatchers.Main) { userActionFab.isEnabled = true }
                    }
                }
            }
        }
    }

    override fun onDestroy() {
        gatewayStatusJob?.cancel()
        vifuAgent?.close()
        super.onDestroy()
    }

    /**
     * Create the `models` directory if not exist.
     */
    private fun ensureModelsDirectory() =
        File(filesDir, DIRECTORY_MODELS).also { directory ->
            check(!directory.exists() || directory.isDirectory) {
                "Model storage path is not a directory"
            }
            check(directory.isDirectory || directory.mkdirs()) {
                "Model storage directory could not be created"
            }
        }

    companion object {
        private val TAG = MainActivity::class.java.simpleName

        private const val DIRECTORY_MODELS = "models"
        private const val FILE_EXTENSION_GGUF = ".gguf"
        private const val DEFAULT_MODEL_NAME = "qwen2.5-0.5b-instruct-q4_k_m.gguf"
        private const val DEFAULT_MODEL_SIZE = 491_400_032L
        private const val MODEL_BUFFER_SIZE = 1024 * 1024
        private const val DEFAULT_MODEL_SHA256 =
            "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db"
        private const val DEFAULT_MODEL_URL =
            "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/" +
                "df5bf01389a39c743ab467d734bf501681e041c5/" +
                "qwen2.5-0.5b-instruct-q4_k_m.gguf"

    }
}

internal fun modelLoadGuidance(message: String, backend: String): String = when {
    "VIFU-LLAMA-BACKEND-" in message ->
        "The optimized backend could not start. Install the baseline build on this device."
    "VIFU-LLAMA-MODEL-001" in message || "VIFU-LLAMA-MODEL-002" in message ->
        "The selected GGUF cannot be read. Choose the model again."
    "VIFU-LLAMA-MODEL-003" in message && backend == "optimized" ->
        "llama.cpp could not load this model with the optimized backend. Try the baseline build."
    "VIFU-LLAMA-MODEL-003" in message ->
        "llama.cpp could not load this GGUF. Check the model format and file."
    else -> "Model setup failed. See the detailed error above."
}

private fun VifuConnectionState.label(): String = when (this) {
    VifuConnectionState.Stopped -> "stopped"
    VifuConnectionState.Connecting -> "connecting"
    VifuConnectionState.Connected -> "connected"
    VifuConnectionState.Reconnecting -> "reconnecting"
    VifuConnectionState.AuthorizationRequired -> "authorization required"
    is VifuConnectionState.Degraded -> "degraded${message?.let { ": $it" }.orEmpty()}"
    is VifuConnectionState.Failed -> "failed${message?.let { ": $it" }.orEmpty()}"
}
