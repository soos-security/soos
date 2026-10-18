package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.soos.remote.cards.BatteryLine
import org.soos.remote.cards.BatteryPower
import org.soos.remote.cards.CardModel
import org.soos.remote.cards.OfflineDuration
import org.soos.remote.net.BatteryView

/** Pure decisions of the battery and offline-history cards (GitHub #439, matrix ANA24). */
class CardModelTest {
    private fun present(percent: Int?, charge: String, externalPower: Boolean? = null) =
        BatteryView("present", percent, charge, externalPower)

    @Test
    fun batteryLineFollowsTheState() {
        assertEquals(BatteryLine.DISABLED, CardModel.batteryLine(BatteryView("disabled", null, null, null)))
        assertEquals(BatteryLine.UNAVAILABLE, CardModel.batteryLine(BatteryView("unavailable", null, null, null)))
        assertEquals(BatteryLine.NO_BATTERY, CardModel.batteryLine(BatteryView("no_battery", null, null, null)))
        assertEquals(BatteryLine.LEVEL, CardModel.batteryLine(present(0, "discharging")))
        assertEquals(BatteryLine.LEVEL, CardModel.batteryLine(present(100, "full")))
        assertEquals(BatteryLine.LEVEL_UNKNOWN, CardModel.batteryLine(present(null, "unknown")))
    }

    @Test
    fun batteryPowerFollowsTheWebPage() {
        assertEquals(BatteryPower.CHARGING, CardModel.batteryPower(present(50, "charging")))
        assertEquals(BatteryPower.ON_BATTERY, CardModel.batteryPower(present(50, "discharging")))
        assertEquals(BatteryPower.FULL, CardModel.batteryPower(present(100, "full")))
        assertEquals(BatteryPower.PLUGGED_IN_NOT_CHARGING, CardModel.batteryPower(present(80, "not_charging")))
        // The mains line only completes an unknown charge state (app.js `batteryText`).
        assertEquals(BatteryPower.PLUGGED_IN, CardModel.batteryPower(present(80, "unknown", true)))
        assertEquals(BatteryPower.ON_BATTERY, CardModel.batteryPower(present(80, "discharging", true)))
        assertNull(CardModel.batteryPower(present(80, "unknown", false)))
        assertNull(CardModel.batteryPower(present(80, "unknown", null)))
        for (state in listOf("disabled", "unavailable", "no_battery")) {
            assertNull(state, CardModel.batteryPower(BatteryView(state, null, null, null)))
        }
    }

    @Test
    fun offlineDurationBoundaries() {
        assertEquals(OfflineDuration.UnderMinute, CardModel.offlineDuration(0))
        assertEquals(OfflineDuration.UnderMinute, CardModel.offlineDuration(59_999))
        assertEquals(OfflineDuration.UnderMinute, CardModel.offlineDuration(-1))
        assertEquals(OfflineDuration.Minutes(1), CardModel.offlineDuration(60_000))
        assertEquals(OfflineDuration.Minutes(59), CardModel.offlineDuration(3_599_999))
        assertEquals(OfflineDuration.Hours(1), CardModel.offlineDuration(3_600_000))
        assertEquals(OfflineDuration.HoursMinutes(1, 1), CardModel.offlineDuration(3_660_000))
        assertEquals(OfflineDuration.HoursMinutes(47, 59), CardModel.offlineDuration(172_799_999))
        assertEquals(OfflineDuration.Days(2), CardModel.offlineDuration(172_800_000))
        assertEquals(OfflineDuration.Days(2), CardModel.offlineDuration(259_199_999))
        assertEquals(OfflineDuration.Days(3), CardModel.offlineDuration(259_200_000))
        assertEquals(OfflineDuration.Days(Long.MAX_VALUE / 86_400_000), CardModel.offlineDuration(Long.MAX_VALUE))
    }
}
