package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.soos.remote.tour.TourController
import org.soos.remote.tour.TourState
import org.soos.remote.tour.TourStep
import org.soos.remote.tour.TourStore

/** First-run feature tour (GitHub #427, matrix ATR1): the step model and every transition. */
class TourControllerTest {
    private class FakeStore(seen: Boolean = false) : TourStore {
        var reads = 0
        var writes = 0
        private var value = seen

        override var tourSeen: Boolean
            get() {
                reads += 1
                return value
            }
            set(v) {
                writes += 1
                value = v
            }

        fun peek(): Boolean = value
    }

    private fun opened(store: FakeStore): Pair<TourController, TourState> {
        val controller = TourController(store)
        val state = controller.offer(TourState(), statusShown = true)
        assertTrue(state.open)
        return controller to state
    }

    @Test
    fun stepsAreTheIntroductionThenTheFeaturesInOrder() {
        assertEquals(
            listOf(
                TourStep.INTRO,
                TourStep.PHONE_COMPANION,
                TourStep.BATTERY,
                TourStep.REMOTE_LOCK,
                TourStep.INTERNET_ACCESS,
                TourStep.FAILED_PASSWORDS,
                TourStep.PUSH_ALERTS,
                TourStep.CAMERA_VIEW,
                TourStep.OFFLINE_ALERTS,
                TourStep.PC_OFFLINE_ALERTS,
            ),
            TourStep.entries.toList(),
        )
        assertEquals(10, TourState.TOTAL)
        assertEquals(9, TourState.LAST)
        val shared = TourStep.entries.filter { it.sharedWithWebTour }
        assertEquals(8, shared.size)
        assertFalse(TourStep.INTRO.sharedWithWebTour)
        assertFalse(TourStep.PC_OFFLINE_ALERTS.sharedWithWebTour)
    }

    @Test
    fun closedByDefault() {
        val state = TourState()
        assertFalse(state.open)
        assertFalse(state.offered)
        assertEquals(0, state.index)
        assertEquals(TourStep.INTRO, state.step)
        assertEquals(1, state.number)
        assertTrue(state.isFirst)
        assertFalse(state.isLast)
    }

    @Test
    fun neverOfferedBeforeTheStatusViewIsShown() {
        val store = FakeStore()
        val controller = TourController(store)
        val start = TourState()
        assertSame(start, controller.offer(start, statusShown = false))
        assertEquals(0, store.reads)
        assertEquals(0, store.writes)
    }

    @Test
    fun firstRunOpensOnTheIntroductionWithoutWriting() {
        val store = FakeStore()
        val (_, state) = opened(store)
        assertEquals(0, state.index)
        assertTrue(state.offered)
        assertEquals(TourStep.INTRO, state.step)
        assertEquals(0, store.writes)
        assertFalse(store.peek())
    }

    @Test
    fun seenTourIsNotOfferedAgain() {
        val store = FakeStore(seen = true)
        val controller = TourController(store)
        val state = controller.offer(TourState(), statusShown = true)
        assertFalse(state.open)
        assertTrue(state.offered)
        assertEquals(0, store.writes)
    }

    @Test
    fun offeredOncePerScreenLifetime() {
        val store = FakeStore()
        val (controller, open) = opened(store)
        // A later status refresh while the tour is open changes nothing.
        assertSame(open, controller.offer(open, statusShown = true))
        val closed = controller.close(open)
        val reads = store.reads
        // Not offered again after it was closed, and the store is not read again.
        assertSame(closed, controller.offer(closed, statusShown = true))
        assertEquals(reads, store.reads)
        assertFalse(closed.open)
    }

    @Test
    fun nextWalksEveryStepAndDoneClosesAndRemembers() {
        val store = FakeStore()
        var (controller, state) = opened(store)
        for (expected in 1..TourState.LAST) {
            state = controller.next(state)
            assertTrue(state.open)
            assertEquals(expected, state.index)
            assertEquals(TourStep.entries[expected], state.step)
            assertEquals(expected + 1, state.number)
        }
        assertTrue(state.isLast)
        assertFalse(state.isFirst)
        assertEquals(TourStep.PC_OFFLINE_ALERTS, state.step)
        assertEquals(0, store.writes)
        // Done.
        state = controller.next(state)
        assertFalse(state.open)
        assertEquals(0, state.index)
        assertTrue(state.offered)
        assertEquals(1, store.writes)
        assertTrue(store.peek())
    }

    @Test
    fun backGoesToThePreviousStepAndStopsAtTheIntroduction() {
        val store = FakeStore()
        var (controller, state) = opened(store)
        assertSame(state, controller.back(state))
        state = controller.next(controller.next(state))
        assertEquals(2, state.index)
        state = controller.back(state)
        assertEquals(1, state.index)
        state = controller.back(state)
        assertEquals(0, state.index)
        assertTrue(state.open)
        assertSame(state, controller.back(state))
        assertEquals(0, store.writes)
    }

    @Test
    fun skipClosesFromAnyStepAndRemembers() {
        for (steps in 0..TourState.LAST) {
            val store = FakeStore()
            var (controller, state) = opened(store)
            repeat(steps) { state = controller.next(state) }
            assertEquals(steps, state.index)
            state = controller.close(state)
            assertFalse(state.open)
            assertEquals(0, state.index)
            assertEquals(1, store.writes)
            assertTrue(store.peek())
        }
    }

    @Test
    fun replayReopensOnTheIntroductionEvenWhenSeen() {
        val store = FakeStore(seen = true)
        val controller = TourController(store)
        val closed = controller.offer(TourState(), statusShown = true)
        assertFalse(closed.open)
        val replayed = controller.replay(closed)
        assertTrue(replayed.open)
        assertEquals(0, replayed.index)
        assertTrue(replayed.offered)
        val moved = controller.replay(controller.next(controller.next(replayed)))
        assertEquals(0, moved.index)
        assertEquals(0, store.writes)
        // Closing a replayed tour writes the same flag again; nothing else is stored.
        assertFalse(controller.close(moved).open)
        assertEquals(1, store.writes)
        assertTrue(store.peek())
    }

    @Test
    fun aClosedTourIgnoresNavigationAndNeverWrites() {
        val store = FakeStore()
        val controller = TourController(store)
        val closed = TourState()
        assertSame(closed, controller.next(closed))
        assertSame(closed, controller.back(closed))
        assertSame(closed, controller.close(closed))
        assertEquals(0, store.writes)
        assertEquals(0, store.reads)
    }

    @Test
    fun anOutOfRangeIndexStillNamesAStep() {
        assertEquals(TourStep.INTRO, TourState(open = true, index = -3).step)
        assertEquals(TourStep.PC_OFFLINE_ALERTS, TourState(open = true, index = 42).step)
        assertTrue(TourState(open = true, index = 42).isLast)
    }
}
