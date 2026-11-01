package android.util

object Log {
    private fun write(tag: String, message: String?, throwable: Throwable? = null): Int {
        System.err.println("$tag: $message")
        throwable?.printStackTrace(System.err)
        return 0
    }
    @JvmStatic fun d(tag: String, message: String?) = write(tag,message)
    @JvmStatic fun i(tag: String, message: String?) = write(tag,message)
    @JvmStatic fun w(tag: String, message: String?) = write(tag,message)
    @JvmStatic fun e(tag: String, message: String?) = write(tag,message)
    @JvmStatic fun v(tag: String, message: String?) = write(tag,message)
    @JvmStatic fun d(tag: String, message: String?, error: Throwable?) = write(tag,message,error)
    @JvmStatic fun i(tag: String, message: String?, error: Throwable?) = write(tag,message,error)
    @JvmStatic fun w(tag: String, message: String?, error: Throwable?) = write(tag,message,error)
    @JvmStatic fun e(tag: String, message: String?, error: Throwable?) = write(tag,message,error)
    @JvmStatic fun v(tag: String, message: String?, error: Throwable?) = write(tag,message,error)
    @JvmStatic fun getStackTraceString(error: Throwable?): String = error?.stackTraceToString().orEmpty()
}
