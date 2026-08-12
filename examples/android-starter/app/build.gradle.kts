import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.jetbrains.kotlin.android)
}

val vifuProperties = Properties().apply {
    rootProject.file("vifu.properties").takeIf { it.isFile }?.inputStream()?.use { load(it) }
}

val vifuBackend = providers.gradleProperty("vifuBackend").orNull ?: "optimized"
val vifuArtifact = when (vifuBackend) {
    "optimized" -> "vifu-android-llama"
    "baseline" -> "vifu-android-llama-baseline"
    else -> error("vifuBackend must be optimized or baseline")
}
val vifuWhisper = providers.gradleProperty("vifuWhisper").orNull?.toBooleanStrictOrNull() ?: false
val vifuVersion = providers.gradleProperty("vifuVersion").orNull ?: libs.versions.vifu.get()

fun buildConfigString(value: String): String =
    "\"${value.replace("\\", "\\\\").replace("\"", "\\\"")}\""

android {
    namespace = "com.example.llama"
    compileSdk = providers.gradleProperty("VIFU_ANDROID_COMPILE_SDK").map(String::toInt).getOrElse(36)

    defaultConfig {
        applicationId = "dev.vifu.android.starter"

        minSdk = 33
        targetSdk = providers.gradleProperty("VIFU_ANDROID_TARGET_SDK").map(String::toInt).getOrElse(36)

        versionCode = 1
        versionName = "1.0"

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

    buildTypes {
        debug {
            isMinifyEnabled = false
            isShrinkResources = false
        }
        release {
            isMinifyEnabled = true
            isShrinkResources = true
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
