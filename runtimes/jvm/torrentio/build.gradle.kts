plugins { kotlin("jvm") }
dependencies {
    compileOnly("com.github.recloudstream.cloudstream:library-jvm:master-5024ace292-1@jar")
    compileOnly("com.github.Blatzar:NiceHttp:0.4.18")
    compileOnly("com.fasterxml.jackson.module:jackson-module-kotlin:2.13.1")
}
kotlin { jvmToolchain(21) }
tasks.jar { archiveFileName.set("torrentio.jar") }
