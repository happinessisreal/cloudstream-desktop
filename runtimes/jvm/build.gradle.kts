plugins {
    kotlin("jvm") version "2.4.0"
    application
}

allprojects {
    repositories { mavenCentral(); google(); maven("https://jitpack.io") }
    repositories {
        exclusiveContent {
            forRepository {
                maven {
                    name = "pinnedCloudstreamArtifact"
                    url = uri("https://jitpack.io")
                    metadataSources { artifact() }
                }
            }
            filter { includeModule("com.github.recloudstream.cloudstream", "library-jvm") }
        }
    }
}

// Immutable JitPack build for upstream commit 5024ace2922a148d38df2a816b483b529789cf58.
val cloudstreamVersion = "master-5024ace292-1"
dependencies {
    // Upstream's immutable snapshot POM incorrectly declares master-SNAPSHOT.
    // Select the exact JAR and explicitly reproduce its published dependencies.
    implementation("com.github.recloudstream.cloudstream:library-jvm:$cloudstreamVersion@jar")
    implementation("androidx.annotation:annotation-jvm:1.10.0")
    implementation("org.jsoup:jsoup:1.22.1")
    implementation("org.jetbrains.kotlinx:atomicfu:0.33.0")
    implementation("org.jetbrains.kotlinx:kotlinx-datetime:0.8.0")
    implementation("org.jetbrains.kotlinx:kotlinx-io-core:0.9.1")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.11.0")
    implementation("com.fleeksoft.ksoup:ksoup:0.2.6")
    implementation("io.ktor:ktor-http:3.5.0")
    implementation("com.github.Blatzar:NiceHttp:0.4.18")
    implementation("org.mozilla:rhino:1.8.1")
    implementation("dev.whyoleg.cryptography:cryptography-core:0.6.0")
    implementation("dev.whyoleg.cryptography:cryptography-provider-optimal:0.6.0")
    implementation("org.jetbrains.kotlin:kotlin-reflect:2.4.0")
    implementation("com.github.teamnewpipe:NewPipeExtractor:v0.26.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.11.0")
    implementation("com.fasterxml.jackson.module:jackson-module-kotlin:2.13.1")
    implementation("org.ow2.asm:asm:9.9.1")
    implementation("de.femtopedia.dex2jar:dex-translator:2.4.38")
    testImplementation(kotlin("test-junit5"))
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}
kotlin {
    jvmToolchain(21)
    compilerOptions { optIn.addAll("com.lagradost.cloudstream3.InternalAPI", "com.lagradost.cloudstream3.Prerelease") }
}
application { mainClass.set("dev.cloudstream.worker.MainKt") }
tasks.test { useJUnitPlatform() }
tasks.jar {
    archiveFileName.set("worker.jar")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    manifest { attributes["Main-Class"] = "dev.cloudstream.worker.MainKt" }
    from({ configurations.runtimeClasspath.get().map { if (it.isDirectory) it else zipTree(it) } })
    exclude("META-INF/*.SF", "META-INF/*.RSA", "META-INF/*.DSA", "module-info.class", "META-INF/versions/**/module-info.class")
}
