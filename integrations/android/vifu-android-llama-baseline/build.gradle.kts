import com.vanniktech.maven.publish.AndroidSingleVariantLibrary
import com.vanniktech.maven.publish.JavadocJar
import com.vanniktech.maven.publish.SourcesJar

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.maven.publish)
}

val repositoryRoot = rootProject.layout.projectDirectory.dir("../..").asFile
val generatedVifu = layout.buildDirectory.dir("generated/vifu")
val sharedKotlin = rootProject.layout.projectDirectory.dir("llama-shared/src/main/kotlin")

val generateVifuAndroidLlamaBaseline by tasks.registering(Exec::class) {
    group = "build"
    description = "Builds the baseline arm64 Vifu llama provider and Kotlin bindings."
    inputs.files(
        fileTree(repositoryRoot.resolve("crates")) {
            include("**/*.rs", "**/Cargo.toml", "**/uniffi.toml")
        },
        repositoryRoot.resolve("Cargo.lock"),
        repositoryRoot.resolve("scripts/build-android-package.sh"),
        fileTree(repositoryRoot.resolve("scripts/cmake")),
    )
    outputs.dir(generatedVifu)
    environment("VIFU_ANDROID_DIST_DIR", generatedVifu.get().asFile.absolutePath)
    environment("CARGO_TARGET_DIR", repositoryRoot.resolve("target/vifu-android-llama-baseline").absolutePath)
    commandLine(
        repositoryRoot.resolve("scripts/build-android-package.sh").absolutePath,
        "--module",
        "llama",
        "--profile",
        "baseline",
    )
}

android {
    namespace = "dev.vifu.android.llama.baseline"
    compileSdk = providers.gradleProperty("VIFU_ANDROID_COMPILE_SDK").map(String::toInt).getOrElse(36)
    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
        ndk { abiFilters += "arm64-v8a" }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlin { jvmToolchain(17) }
    sourceSets.named("main") {
        kotlin.srcDir(sharedKotlin)
        kotlin.srcDir(generatedVifu.map { it.dir("src/main/kotlin") })
        jniLibs.srcDir(generatedVifu.map { it.dir("src/main/jniLibs") })
    }
}

tasks.named("preBuild").configure { dependsOn(generateVifuAndroidLlamaBaseline) }

dependencies {
    api(project(":vifu-android-core"))
    implementation(libs.androidx.annotation)
    implementation(libs.jna) { artifact { type = "aar" } }
    testImplementation(libs.junit)
}

mavenPublishing {
    configure(
        AndroidSingleVariantLibrary(
            javadocJar = JavadocJar.Empty(),
            sourcesJar = SourcesJar.Sources(),
            variant = "release",
        ),
    )
    coordinates(project.group.toString(), "vifu-android-llama-baseline", project.version.toString())
    publishToMavenCentral(automaticRelease = true)
    if (providers.gradleProperty("signingInMemoryKey").isPresent) signAllPublications()
    pom {
        name.set("Vifu Android Llama Baseline")
        description.set("Optional conservative ARMv8 llama.cpp provider for Vifu Android.")
        inceptionYear.set("2026")
        url.set("https://github.com/vifudotdev/vifu")
        licenses { license {
            name.set("The Apache License, Version 2.0")
            url.set("https://www.apache.org/licenses/LICENSE-2.0.txt")
            distribution.set("repo")
        } }
        developers { developer { id.set("vifu"); name.set("Vifu Contributors") } }
        scm {
            url.set("https://github.com/vifudotdev/vifu")
            connection.set("scm:git:git://github.com/vifudotdev/vifu.git")
            developerConnection.set("scm:git:ssh://git@github.com/vifudotdev/vifu.git")
        }
    }
}
