import org.gradle.api.DefaultTask
import org.gradle.api.file.ConfigurableFileCollection
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.provider.ListProperty
import org.gradle.api.provider.Property
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.InputFiles
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.OutputFile
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction
import org.gradle.process.ExecOperations
import javax.inject.Inject

plugins {
    alias(libs.plugins.android.library)
}

val rustRoot = rootProject.layout.projectDirectory.dir("../core")
val rustProfile = providers.gradleProperty("lounge.rust.profile").getOrElse("release")
val rustAbis = providers.gradleProperty("lounge.rust.abis").getOrElse("arm64-v8a").split(",").map { it.trim() }
val cargoHome = System.getenv("CARGO_HOME") ?: "${System.getProperty("user.home")}/.cargo"
val cargoBin = "$cargoHome/bin/cargo"
val ndkDir = providers.gradleProperty("lounge.ndk.dir").orNull
    ?: (System.getenv("ANDROID_NDK_HOME") ?: "${System.getProperty("user.home")}/Android/Sdk/ndk/27.2.12479018")

/** Compiles liblounge_core.so for each ABI with cargo-ndk (16 KB page aligned by NDK r27+). */
abstract class CargoNdkBuild : DefaultTask() {
    @get:Inject abstract val execOps: ExecOperations
    @get:InputFiles @get:PathSensitive(PathSensitivity.RELATIVE) abstract val sources: ConfigurableFileCollection
    @get:Input abstract val cargo: Property<String>
    @get:Input abstract val workspace: Property<String>
    @get:Input abstract val abis: ListProperty<String>
    @get:Input abstract val release: Property<Boolean>
    @get:Input abstract val ndk: Property<String>
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun run() {
        val out = outputDir.get().asFile
        out.mkdirs()
        val args = mutableListOf(cargo.get(), "ndk", "--platform", "28", "-o", out.absolutePath)
        abis.get().forEach { args += listOf("-t", it) }
        args += listOf("build", "-p", "lounge-core")
        if (release.get()) args += "--release"
        execOps.exec {
            workingDir = File(workspace.get())
            environment("ANDROID_NDK_HOME", ndk.get())
            // libjpeg-turbo is built from source by cmake, which needs the NDK's toolchain file.
            environment("CMAKE_TOOLCHAIN_FILE", "${ndk.get()}/build/cmake/android.toolchain.cmake")
            // C compiled for 32-bit ARM gets NEON; the default for the target leaves it out.
            environment("CFLAGS_armv7_linux_androideabi", "-mfpu=neon-vfpv4")
            commandLine(args)
        }
    }
}

/** Builds the host cdylib so uniffi-bindgen can read the exported interface from it. */
abstract class CargoHostBuild : DefaultTask() {
    @get:Inject abstract val execOps: ExecOperations
    @get:InputFiles @get:PathSensitive(PathSensitivity.RELATIVE) abstract val sources: ConfigurableFileCollection
    @get:Input abstract val cargo: Property<String>
    @get:Input abstract val workspace: Property<String>
    @get:OutputFile abstract val library: RegularFileProperty

    @TaskAction
    fun run() {
        execOps.exec {
            workingDir = File(workspace.get())
            commandLine(cargo.get(), "build", "-p", "lounge-core", "--profile", "bindgen")
        }
    }
}

/** Generates the Kotlin bindings for the core into a source directory. */
abstract class UniffiBindgen : DefaultTask() {
    @get:Inject abstract val execOps: ExecOperations
    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val library: RegularFileProperty
    @get:InputFile @get:PathSensitive(PathSensitivity.NONE) abstract val config: RegularFileProperty
    @get:Input abstract val cargo: Property<String>
    @get:Input abstract val workspace: Property<String>
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun run() {
        val out = outputDir.get().asFile
        out.deleteRecursively()
        out.mkdirs()
        execOps.exec {
            workingDir = File(workspace.get())
            commandLine(
                cargo.get(), "run", "-q", "-p", "uniffi-bindgen", "--profile", "bindgen", "--",
                "generate",
                library.get().asFile.absolutePath,
                "--language", "kotlin",
                "--out-dir", out.absolutePath,
                "--no-format",
            )
        }
    }
}

val companionRoot = rustRoot.dir("../companion")

val rustSources = files(
    rustRoot.file("Cargo.toml"),
    rustRoot.file("Cargo.lock"),
    rustRoot.file(".cargo/config.toml"),
    rustRoot.file("rust-toolchain.toml"),
    rustRoot.dir("lounge-core"),
    rustRoot.dir("lounge-graph"),
    rustRoot.dir("lounge-store"),
    rustRoot.dir("lounge-artwork"),
    rustRoot.dir("lounge-plugins"),
    rustRoot.dir("lounge-plugin-api"),
    rustRoot.dir("lounge-companion"),
    companionRoot.dir("dist"),
)

// The companion web app is embedded into the core at compile time, so it is built first.
val buildCompanionSpa = tasks.register<Exec>("buildCompanionSpa") {
    group = "rust"
    description = "Build the Svelte companion app (pnpm build) for embedding in the core"
    inputs.files(fileTree(companionRoot.dir("src")), companionRoot.file("package.json"), companionRoot.file("vite.config.ts"), companionRoot.file("index.html"), fileTree(companionRoot.dir("public")))
    outputs.dir(companionRoot.dir("dist"))
    workingDir(companionRoot.asFile)
    val pnpm = providers.gradleProperty("lounge.pnpm").getOrElse("pnpm")
    commandLine(pnpm, "build")
    val nodeModules = companionRoot.dir("node_modules").asFile
    val dist = companionRoot.dir("dist").asFile
    onlyIf {
        when {
            nodeModules.exists() -> true
            dist.exists() -> false
            else -> throw GradleException("companion/dist is missing and companion/node_modules is absent: run `pnpm install` in companion/ first")
        }
    }
}

val cargoBuildAndroid = tasks.register<CargoNdkBuild>("cargoBuildAndroid") {
    group = "rust"
    description = "Compile the Rust core for Android ABIs with cargo-ndk"
    dependsOn(buildCompanionSpa)
    sources.from(rustSources)
    cargo.set(cargoBin)
    workspace.set(rustRoot.asFile.absolutePath)
    abis.set(rustAbis)
    release.set(rustProfile == "release")
    ndk.set(ndkDir)
    outputDir.set(layout.buildDirectory.dir("rust/jniLibs"))
}

val cargoBuildHost = tasks.register<CargoHostBuild>("cargoBuildHost") {
    group = "rust"
    description = "Compile the Rust core for the host so bindings can be generated"
    dependsOn(buildCompanionSpa)
    sources.from(rustSources)
    cargo.set(cargoBin)
    workspace.set(rustRoot.asFile.absolutePath)
    library.set(rustRoot.file("target/bindgen/liblounge_core.so"))
}

val generateUniffiBindings = tasks.register<UniffiBindgen>("generateUniffiBindings") {
    group = "rust"
    description = "Generate the Kotlin bindings for the Rust core"
    library.set(cargoBuildHost.flatMap { it.library })
    config.set(rustRoot.file("lounge-core/uniffi.toml"))
    cargo.set(cargoBin)
    workspace.set(rustRoot.asFile.absolutePath)
    outputDir.set(layout.buildDirectory.dir("generated/uniffi/kotlin"))
}

android {
    namespace = "tv.lounge.core"
    compileSdk = 37

    defaultConfig {
        minSdk = 28
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

androidComponents {
    onVariants { variant ->
        variant.sources.kotlin?.addGeneratedSourceDirectory(generateUniffiBindings, UniffiBindgen::outputDir)
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargoBuildAndroid, CargoNdkBuild::outputDir)
    }
}

dependencies {
    implementation(libs.jna) { artifact { type = "aar" } }
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.androidx.core.ktx)
}
