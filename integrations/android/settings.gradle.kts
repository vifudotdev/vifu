pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "vifu-android"
include(":vifu-android-core")
include(":vifu-android-llama")
include(":vifu-android-llama-baseline")
include(":vifu-android-whisper")
