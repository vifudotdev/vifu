package com.example.llama

import android.net.Uri
import android.os.Bundle
import android.util.Base64
import android.util.Log
import android.widget.EditText
import android.widget.TextView
import android.widget.Toast
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
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
import kotlinx.coroutines.withContext
import java.io.File
import java.io.FileOutputStream
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.StandardCopyOption
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

        // Upon CTA button tapped
        userActionFab.setOnClickListener {
            if (isModelReady) {
                // If model is ready, validate input and send to engine
                handleUserInput()
            } else {
                // Otherwise, prompt user to select a GGUF metadata on the device
                getContent.launch(arrayOf("*/*"))
            }
        }

        ensureModelsDirectory()
            .listFiles { file -> file.isFile && file.extension.equals("gguf", ignoreCase = true) }
            ?.maxByOrNull(File::lastModified)
            ?.let(::loadModelFile)
    }

    private val getContent = registerForActivityResult(
        ActivityResultContracts.OpenDocument()
    ) { uri ->
        Log.i(TAG, "Model file selected")
        uri?.let { handleSelectedModel(it) }
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
        connectVifu(modelFile)
        withContext(Dispatchers.Main) {
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
    private suspend fun connectVifu(modelFile: File) = withContext(Dispatchers.Default) {
        withContext(Dispatchers.Main) { userInputEt.hint = "Loading model..." }
        gatewayStatusJob?.cancel()
        gatewayStatusJob = null
        vifuAgent?.close()
        vifuAgent = null
        val model = VifuLlamaConfig(modelPath = modelFile.absolutePath)
        val hasGatewayConfig =
            BuildConfig.VIFU_SERVER_URL.isNotBlank() && BuildConfig.VIFU_APP_ID.isNotBlank()
        vifuAgent = if (hasGatewayConfig) {
            VifuLlamaAgent.open(
                context = applicationContext,
                connection = VifuConnectionConfig(
                    serverUrl = BuildConfig.VIFU_SERVER_URL,
                    appId = BuildConfig.VIFU_APP_ID,
                    serverCertificateDer = BuildConfig.VIFU_SERVER_CERTIFICATE_DER_BASE64
                        .takeIf(String::isNotBlank)
                        ?.let { Base64.decode(it, Base64.DEFAULT) },
                ),
                model = model,
            ).also { agent ->
                gatewayStatusJob = lifecycleScope.launch {
                    agent.connectionState.collect { state ->
                        vifuStatusTv.text = "Vifu: ${state.label()}"
                    }
                }
            }
        } else {
            VifuLlamaAgent.open(applicationContext, model).also {
                withContext(Dispatchers.Main) { vifuStatusTv.text = "Vifu: local" }
            }
        }
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
                    requireNotNull(vifuAgent) { "Vifu is not connected" }
                        .send(userMessage)
                        .onCompletion { error ->
                            withContext(Dispatchers.Main) {
                                userActionFab.isEnabled = true
                                error?.message?.let {
                                    Toast.makeText(this@MainActivity, it, Toast.LENGTH_LONG).show()
                                }
                            }
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
