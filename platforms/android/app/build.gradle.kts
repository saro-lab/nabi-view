plugins { id("com.android.application") }

val nabiAbi = providers.gradleProperty("nabiAbi").orElse("arm64-v8a").get()
require(nabiAbi in listOf("arm64-v8a", "x86_64", "all")) { "Unsupported ABI: $nabiAbi" }
val nabiAbis = if (nabiAbi == "all") listOf("arm64-v8a", "x86_64") else listOf(nabiAbi)
val nabiNdkHome = providers.environmentVariable("ANDROID_NDK_HOME").orNull
val nabiNdkVersion = nabiNdkHome?.let { directory ->
    file("$directory/source.properties").readLines()
        .firstOrNull { it.substringBefore("=").trim() == "Pkg.Revision" }
        ?.substringAfter("=")?.trim() ?: error("NDK source.properties has no Pkg.Revision")
} ?: "28.2.13676358"
val cargoPackage = file("../../../Cargo.toml").readText().substringBefore("[workspace]")
val nabiVersion = Regex("(?m)^version\\s*=\\s*\"([^\"]+)\"").find(cargoPackage)?.groupValues?.get(1)
    ?: error("Cargo.toml package version is missing")
val nabiVersionCode = providers.environmentVariable("NABI_ANDROID_VERSION_CODE").orElse("1").get().toInt()
require(nabiVersionCode in 1..2100000000) { "NABI_ANDROID_VERSION_CODE must be in 1..2100000000" }
val nabiApplicationId = providers.environmentVariable("NABI_ANDROID_APPLICATION_ID").orElse("dev.nabi.view").get()
require(Regex("[A-Za-z][A-Za-z0-9_]*(\\.[A-Za-z][A-Za-z0-9_]*)+").matches(nabiApplicationId)) {
    "NABI_ANDROID_APPLICATION_ID must be a valid Android application ID"
}
// Passwords are read only from the environment, never Gradle command-line arguments.
val signingValues = listOf("NABI_ANDROID_KEYSTORE", "NABI_ANDROID_KEYSTORE_PASSWORD", "NABI_ANDROID_KEY_ALIAS", "NABI_ANDROID_KEY_PASSWORD")
    .associateWith { providers.environmentVariable(it).orNull?.takeIf(String::isNotEmpty) }
val hasSigning = signingValues.values.all { it != null }
require(hasSigning || signingValues.values.all { it == null }) { "Set all four NABI_ANDROID signing variables, or unset all of them" }
if (hasSigning) require(file(signingValues.getValue("NABI_ANDROID_KEYSTORE")!!).isFile) { "Android signing keystore does not exist" }
android {
    namespace = "dev.nabi.view"
    ndkVersion = nabiNdkVersion
    if (nabiNdkHome != null) ndkPath = nabiNdkHome
    compileSdk = 36
    defaultConfig {
        applicationId = nabiApplicationId
        minSdk = 30
        targetSdk = 36
        versionCode = nabiVersionCode
        versionName = nabiVersion
        ndk { abiFilters += nabiAbis }
    }
    if (hasSigning) {
        signingConfigs.create("release") {
            storeFile = file(signingValues.getValue("NABI_ANDROID_KEYSTORE")!!)
            storePassword = signingValues.getValue("NABI_ANDROID_KEYSTORE_PASSWORD")
            keyAlias = signingValues.getValue("NABI_ANDROID_KEY_ALIAS")
            keyPassword = signingValues.getValue("NABI_ANDROID_KEY_PASSWORD")
        }
        buildTypes.getByName("release").signingConfig = signingConfigs.getByName("release")
    }
    buildTypes.getByName("release").isDebuggable = false
    sourceSets["main"].apply {
        jniLibs.srcDir("../../../target/android/jniLibs")
        assets.srcDir("../../../target/android/assets")
        res.srcDir("../../../target/android/res")
    }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    // AGP >= 8.5.1 aligns uncompressed native libraries for 16 KB devices.
    packaging { jniLibs { useLegacyPackaging = false } }
}
layout.buildDirectory.set(file("../../../target/android/gradle/app"))

val buildRust by tasks.registering(Exec::class) {
    workingDir = file("../../..")
    commandLine("sh", "build/android.sh", "--native-only", "--abi", nabiAbi)
    environment("ANDROID_HOME", android.sdkDirectory.absolutePath)
    environment("ANDROID_NDK_HOME", nabiNdkHome ?: android.sdkDirectory.resolve("ndk/$nabiNdkVersion").absolutePath)
    onlyIf { !providers.gradleProperty("nabiNativePrepared").isPresent }
}
tasks.named("preBuild").configure { dependsOn(buildRust) }
