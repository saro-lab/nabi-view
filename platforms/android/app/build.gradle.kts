plugins { id("com.android.application") }

val nabiAbi = providers.gradleProperty("nabiAbi").orElse("arm64-v8a").get()
require(nabiAbi in listOf("arm64-v8a", "x86_64")) { "Unsupported ABI: $nabiAbi" }
android {
    namespace = "dev.nabi.view"
    compileSdk = 35
    defaultConfig {
        applicationId = "dev.nabi.view"
        minSdk = 30
        targetSdk = 35
        versionCode = 1
        versionName = "1.0.0"
        ndk { abiFilters += nabiAbi }
    }
    sourceSets["main"].apply {
        jniLibs.srcDir("../../../target/android/jniLibs")
        assets.srcDir("../../../target/android/assets")
        res.srcDir("../../../target/android/res")
    }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    packaging { jniLibs { useLegacyPackaging = true } }
}
layout.buildDirectory.set(file("../../../target/android/gradle/app"))

val buildRust by tasks.registering(Exec::class) {
    workingDir = file("../../..")
    commandLine("sh", "build/android.sh", "--native-only", "--abi", nabiAbi)
    environment("ANDROID_HOME", android.sdkDirectory.absolutePath)
    onlyIf { !providers.gradleProperty("nabiNativePrepared").isPresent }
}
tasks.named("preBuild").configure { dependsOn(buildRust) }
