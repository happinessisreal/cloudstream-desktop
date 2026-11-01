package android.content

/** Nominal load(context) argument for data-only providers. No Android services
 * are emulated. The package scanner rejects calls into this type before load. */
class Context internal constructor()
