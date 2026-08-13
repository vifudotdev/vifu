import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.jetbrains.kotlin.android)
}

val useBuildTimePairing = providers.gradleProperty("vifuUseBuildTimePairing")
    .map(String::toBoolean)
    .getOrElse(false)
val vifuProperties = Properties().apply {
    if (useBuildTimePairing) {
        rootProject.file("vifu.properties").takeIf { it.isFile }?.inputStream()?.use { load(it) }
    }
}

val vifuBackend = providers.gradleProperty("vifuBackend").orNull ?: "optimized"
val starterApplicationId = when (vifuBackend) {
    "optimized" -> "dev.vifu.android.starter.optimized"
    "baseline" -> "dev.vifu.android.starter.baseline"
    else -> error("vifuBackend must be optimized or baseline")
}
val starterApplicationName = when (vifuBackend) {
    "optimized" -> "Vifu Starter Optimized"
    "baseline" -> "Vifu Starter Baseline"
    else -> error("vifuBackend must be optimized or baseline")
}
val vifuArtifact = when (vifuBackend) {
    "optimized" -> "vifu-android-llama"
    "baseline" -> "vifu-android-llama-baseline"
    else -> error("vifuBackend must be optimized or baseline")
}
val vifuWhisper = providers.gradleProperty("vifuWhisper").orNull?.toBooleanStrictOrNull() ?: false
val vifuVersion = providers.gradleProperty("vifuVersion").orNull ?: libs.versions.vifu.get()
val starterVersionName = providers.gradleProperty("starterVersionName").orNull ?: "0.1.1"
val starterVersionCode = providers.gradleProperty("starterVersionCode").orNull?.toIntOrNull() ?: 1
val releaseKeystoreFile = providers.environmentVariable("VIFU_ANDROID_STARTER_KEYSTORE").orNull
val releaseKeystorePassword = providers.environmentVariable("VIFU_ANDROID_STARTER_KEYSTORE_PASSWORD").orNull
val releaseKeyAlias = providers.environmentVariable("VIFU_ANDROID_STARTER_KEY_ALIAS").orNull
val releaseKeyPassword = providers.environmentVariable("VIFU_ANDROID_STARTER_KEY_PASSWORD").orNull

fun buildConfigString(value: String): String =
    "\"${value.replace("\\", "\\\\").replace("\"", "\\\"")}\""

android {
    namespace = "com.example.llama"
    compileSdk = providers.gradleProperty("VIFU_ANDROID_COMPILE_SDK").map(String::toInt).getOrElse(36)

    defaultConfig {
        applicationId = starterApplicationId

        minSdk = 33
        targetSdk = providers.gradleProperty("VIFU_ANDROID_TARGET_SDK").map(String::toInt).getOrElse(36)

        versionCode = starterVersionCode
        versionName = starterVersionName
        manifestPlaceholders["vifuAppName"] = starterApplicationName

        ndk { abiFilters += "arm64-v8a" }

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        vectorDrawables {
            useSupportLibrary = true
        }
        buildConfigField("String", "VIFU_SERVER_URL", buildConfigString(vifuProperties.getProperty("serverUrl", "")))
        buildConfigField("String", "VIFU_APP_ID", buildConfigString(vifuProperties.getProperty("appId", "")))
        buildConfigField("String", "VIFU_BACKEND", buildConfigString(vifuBackend))
        buildConfigField(
            "String",
            "VIFU_SERVER_CERTIFICATE_DER_BASE64",
            buildConfigString(vifuProperties.getProperty("serverCertificateDerBase64", "")),
        )
    }

    signingConfigs {
        if (
            releaseKeystoreFile != null &&
            releaseKeystorePassword != null &&
            releaseKeyAlias != null &&
            releaseKeyPassword != null
        ) {
            create("vifuRelease") {
                storeFile = file(releaseKeystoreFile)
                storePassword = releaseKeystorePassword
                keyAlias = releaseKeyAlias
                keyPassword = releaseKeyPassword
            }
        }
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
            isShrinkResources = false
        }
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            signingConfig = signingConfigs.findByName("vifuRelease")
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlin { jvmToolchain(17) }
    buildFeatures { buildConfig = true }
    packaging { jniLibs { useLegacyPackaging = true } }
}

dependencies {
    implementation(libs.bundles.androidx)
    implementation(libs.material)

    implementation("dev.vifu:$vifuArtifact:$vifuVersion")
    if (vifuWhisper) {
        implementation("dev.vifu:vifu-android-whisper:$vifuVersion")
    }

    testImplementation(libs.junit)
    androidTestImplementation(libs.androidx.junit)
    androidTestImplementation(libs.androidx.espresso.core)
}
