// soos native Android app (GitHub #382, ADR 2026-10-13 "Native Android App for
// `soos-remote` (Kotlin and Jetpack Compose)"). Not part of the Cargo workspace.
// Every dependency is pinned in gradle/libs.versions.toml and verified against
// gradle/verification-metadata.xml; build through scripts/android_build.sh.

pluginManagement {
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        mavenCentral()
    }
}

rootProject.name = "soos-android"
include(":app")
