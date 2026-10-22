plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.ksp)
    alias(libs.plugins.hilt)
}

android {
    namespace = "tv.lounge.launcher"
    compileSdk = 37

    defaultConfig {
        applicationId = "tv.lounge.launcher"
        minSdk = 28
        targetSdk = 37
        versionCode = 6
        versionName = "0.3.1"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a") }
    }

    // A real release key comes from ~/.gradle/gradle.properties (lounge.storeFile, storePassword,
    // keyAlias, keyPassword); without one, release builds sign with the debug key.
    val releaseStore = providers.gradleProperty("lounge.storeFile").orNull
    signingConfigs {
        if (releaseStore != null) {
            create("release") {
                storeFile = file(releaseStore)
                storePassword = providers.gradleProperty("lounge.storePassword").orNull
                keyAlias = providers.gradleProperty("lounge.keyAlias").orNull
                keyPassword = providers.gradleProperty("lounge.keyPassword").orNull
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
        debug {
            applicationIdSuffix = ".debug"
        }
        // Release without R8, for capturing the baseline profile (tools/baseline-profile.sh).
        create("profile") {
            initWith(getByName("release"))
            isMinifyEnabled = false
            isShrinkResources = false
            applicationIdSuffix = ".profile"
            matchingFallbacks += listOf("release")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }

    buildFeatures { compose = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        jniLibs { useLegacyPackaging = false }
        resources.excludes += setOf("META-INF/AL2.0", "META-INF/LGPL2.1")
    }
}

composeCompiler {
    stabilityConfigurationFiles.add(rootProject.layout.projectDirectory.file("compose-stability.conf"))
}

dependencies {
    implementation(project(":core:bindings"))
    implementation(project(":core:design"))
    implementation(project(":core:platform"))
    implementation(project(":collector:apps"))
    implementation(project(":collector:tvprovider"))
    implementation(project(":collector:media"))
    implementation(project(":collector:inputs"))
    implementation(project(":feature:home"))
    implementation(project(":power"))
    implementation(project(":update"))

    val bom = platform(libs.compose.bom)
    implementation(bom)
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.runtime)
    implementation(libs.tv.material)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.process)
    implementation(libs.androidx.core.ktx)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.coil.compose)
    implementation(libs.hilt.android)
    implementation(libs.androidx.profileinstaller)
    implementation(libs.androidx.savedstate)
    // Composition tracing in Perfetto, only in the profile build (names every composable in the trace).
    "profileImplementation"(libs.compose.runtime.tracing)
    "profileImplementation"("androidx.tracing:tracing-perfetto-binary:1.0.0")
    ksp(libs.hilt.compiler)
    debugImplementation(libs.compose.ui.tooling)
}
