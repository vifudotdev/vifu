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
        if (providers.gradleProperty("vifuUseMavenLocal").orNull == "true") {
            mavenLocal()
        }
    }
}

rootProject.name = "VifuAndroidStarter"
include(":app")

if (providers.gradleProperty("vifuUseLocalCheckout").orNull == "true") {
    includeBuild("../../integrations/android") {
        dependencySubstitution {
            substitute(module("dev.vifu:vifu-android-core")).using(project(":vifu-android-core"))
            substitute(module("dev.vifu:vifu-android-llama")).using(project(":vifu-android-llama"))
            substitute(module("dev.vifu:vifu-android-llama-baseline"))
                .using(project(":vifu-android-llama-baseline"))
            substitute(module("dev.vifu:vifu-android-whisper"))
                .using(project(":vifu-android-whisper"))
        }
    }
}
