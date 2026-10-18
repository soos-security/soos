package org.soos.remote.tour

/*
 * First-run feature tour of the app (GitHub #427, PR 3 of #363; ADR 2026-10-18 "Phone
 * Onboarding Tour in the Native Android App"). Pure Kotlin: the steps, the state and its
 * transitions. The texts are strings.xml resources (ui/TourCard.kt). The tour makes no request,
 * switches nothing and reads no server or user data; the only stored datum is one Boolean,
 * "the tour was closed once", through TourStore.
 */

/**
 * The steps in order: the introduction, then one per feature. [sharedWithWebTour] marks the
 * steps whose title, explanation and "Off:" line equal the web tour of `app.js`.
 */
enum class TourStep(val sharedWithWebTour: Boolean) {
    INTRO(false),
    PHONE_COMPANION(true),
    BATTERY(true),
    REMOTE_LOCK(true),
    INTERNET_ACCESS(true),
    FAILED_PASSWORDS(true),
    PUSH_ALERTS(true),
    CAMERA_VIEW(true),
    OFFLINE_ALERTS(true),
    PC_OFFLINE_ALERTS(false),
}

/** Remembers that the tour was closed once (implemented by the private settings store). */
interface TourStore {
    var tourSeen: Boolean
}

/**
 * What the tour shows: [open] or not, the step [index] (0 is the introduction) and whether the
 * first-run tour was already [offered] while this screen lives.
 */
data class TourState(val open: Boolean = false, val index: Int = 0, val offered: Boolean = false) {
    /** The step shown; an out-of-range index is clamped, so it always names a step. */
    val step: TourStep
        get() = TourStep.entries[index.coerceIn(0, LAST)]

    /** 1 for the introduction, [TOTAL] for the last step. */
    val number: Int
        get() = index.coerceIn(0, LAST) + 1

    val isFirst: Boolean
        get() = index <= 0

    val isLast: Boolean
        get() = index >= LAST

    companion object {
        val TOTAL: Int = TourStep.entries.size
        val LAST: Int = TOTAL - 1
    }
}

/** The transitions; stateless besides the store, which only [close] writes. */
class TourController(private val store: TourStore) {
    /** Opens the first-run tour once, after a status view is shown, unless it was closed before. */
    fun offer(state: TourState, statusShown: Boolean): TourState {
        if (!statusShown || state.offered || state.open) return state
        return TourState(open = !store.tourSeen, index = 0, offered = true)
    }

    /** Feature tour: opens on the introduction, whether or not it was seen. */
    fun replay(state: TourState): TourState = state.copy(open = true, index = 0, offered = true)

    /** Next; Done on the last step closes the tour. */
    fun next(state: TourState): TourState = when {
        !state.open -> state
        state.isLast -> close(state)
        else -> state.copy(index = state.index + 1)
    }

    /** Back to the previous step; nothing before the introduction. */
    fun back(state: TourState): TourState =
        if (!state.open || state.isFirst) state else state.copy(index = state.index - 1)

    /** Skip tour, system back or Done: closes and remembers. */
    fun close(state: TourState): TourState {
        if (!state.open) return state
        store.tourSeen = true
        return TourState(open = false, index = 0, offered = true)
    }
}
