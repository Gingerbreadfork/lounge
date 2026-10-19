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
        google()
        mavenCentral()
    }
}

rootProject.name = "lounge"

include(":app")
include(":core:bindings")
include(":core:design")
include(":core:platform")
include(":collector:apps")
include(":collector:tvprovider")
include(":collector:media")
include(":collector:inputs")
include(":feature:home")
include(":power")
