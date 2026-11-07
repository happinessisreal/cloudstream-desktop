plugins { kotlin("jvm") }
dependencies {
    compileOnly("com.github.recloudstream.cloudstream:library-jvm:master-5024ace292-1@jar")
    compileOnly("com.github.Blatzar:NiceHttp:0.4.18")
    compileOnly("com.fasterxml.jackson.module:jackson-module-kotlin:2.13.1")

    testImplementation(rootProject)
    testImplementation(kotlin("test-junit5"))
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}
kotlin {
    jvmToolchain(21)
    compilerOptions { optIn.addAll("com.lagradost.cloudstream3.InternalAPI", "com.lagradost.cloudstream3.Prerelease") }
}
tasks.jar { archiveFileName.set("torrentio.jar") }
tasks.test { useJUnitPlatform() }
