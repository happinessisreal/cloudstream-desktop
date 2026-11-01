plugins { kotlin("jvm") }
dependencies { compileOnly("com.github.recloudstream.cloudstream:library-jvm:master-5024ace292-1@jar") }
kotlin { jvmToolchain(21) }
tasks.jar { archiveFileName.set("desktop-sample.jar") }
