import java.security.MessageDigest
import java.util.Properties

plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.jetbrains.kotlin.android) apply false
}

tasks.register("configureVifu") {
    group = "vifu"
    description = "Creates the ignored Android Vifu connection file from ~/.vifu/config.toml."
    doLast {
        val appId = providers.gradleProperty("vifuAppId").orNull
            ?: error("Pass the App ID: -PvifuAppId=vifu_app_<64 hex characters>")
        require(Regex("^vifu_app_[0-9a-fA-F]{64}$").matches(appId)) {
            "vifuAppId must use the vifu_app_<64 hex characters> format"
        }

        val vifuHome = File(System.getProperty("user.home"), ".vifu")
        val configFile = File(vifuHome, "config.toml")
        val configServer = configFile.takeIf(File::isFile)?.readLines()
            ?.dropWhile { it.trim() != "[server]" }
            ?.drop(1)
            ?.takeWhile { !it.trim().startsWith("[") }
            ?.firstNotNullOfOrNull { line ->
                Regex("^\\s*address\\s*=\\s*\"([^\"]+)\"").find(line)?.groupValues?.get(1)
            }
        val serverUrl = providers.gradleProperty("vifuServerUrl").orNull ?: configServer
            ?: error("Pass -PvifuServerUrl=https://<reachable-host>:<port> or configure ~/.vifu/config.toml")

        val explicitCertificate = providers.gradleProperty("vifuCertificateFile").orNull?.let(::File)
        val certificateId = MessageDigest.getInstance("SHA-256")
            .digest(serverUrl.toByteArray())
            .take(8)
            .joinToString("") { "%02x".format(it) }
        val certificateFile = explicitCertificate ?: File(vifuHome, "server-$certificateId-cert.der.b64")
        val certificate = certificateFile.takeIf(File::isFile)?.readText()?.trim().orEmpty()

        Properties().apply {
            setProperty("serverUrl", serverUrl)
            setProperty("appId", appId)
            setProperty("serverCertificateDerBase64", certificate)
            rootProject.file("vifu.properties").outputStream().use { store(it, null) }
        }
        logger.lifecycle("Configured the ignored Android Vifu connection file")
    }
}
