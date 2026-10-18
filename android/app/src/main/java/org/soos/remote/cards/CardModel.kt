package org.soos.remote.cards

import org.soos.remote.net.BatteryView

/*
 * Pure decisions of the read-only battery and offline-history cards (GitHub #439, part of #357).
 * Same rules as the web page (`crates/remote/assets/app.js` `batteryText`, `durationText`). The
 * texts are strings.xml resources (ui/PcCards.kt): a server value only selects one of them.
 */

/** The main line of the battery card. */
enum class BatteryLine { DISABLED, UNAVAILABLE, NO_BATTERY, LEVEL, LEVEL_UNKNOWN }

/** The power line of a present battery. */
enum class BatteryPower { CHARGING, ON_BATTERY, FULL, PLUGGED_IN_NOT_CHARGING, PLUGGED_IN }

/** A rounded-down offline duration. */
sealed interface OfflineDuration {
    data object UnderMinute : OfflineDuration

    data class Minutes(val minutes: Long) : OfflineDuration

    data class Hours(val hours: Long) : OfflineDuration

    data class HoursMinutes(val hours: Long, val minutes: Long) : OfflineDuration

    data class Days(val days: Long) : OfflineDuration
}

object CardModel {
    private const val MINUTE_MS = 60_000L
    private const val MINUTES_PER_HOUR = 60L
    private const val HOURS_SHOWN = 48L
    private const val HOURS_PER_DAY = 24L

    fun batteryLine(view: BatteryView): BatteryLine = when (view.state) {
        "disabled" -> BatteryLine.DISABLED
        "no_battery" -> BatteryLine.NO_BATTERY
        "present" -> if (view.percent != null) BatteryLine.LEVEL else BatteryLine.LEVEL_UNKNOWN
        else -> BatteryLine.UNAVAILABLE
    }

    /** `null` when nothing is known about the power (or no battery is present). */
    fun batteryPower(view: BatteryView): BatteryPower? {
        if (view.state != "present") return null
        return when (view.charge) {
            "charging" -> BatteryPower.CHARGING
            "discharging" -> BatteryPower.ON_BATTERY
            "full" -> BatteryPower.FULL
            "not_charging" -> BatteryPower.PLUGGED_IN_NOT_CHARGING
            else -> if (view.externalPower == true) BatteryPower.PLUGGED_IN else null
        }
    }

    /** Under a minute, minutes below an hour, hours (and minutes) below 48 h, else days. */
    fun offlineDuration(durationMs: Long): OfflineDuration {
        if (durationMs < MINUTE_MS) return OfflineDuration.UnderMinute
        val minutes = durationMs / MINUTE_MS
        if (minutes < MINUTES_PER_HOUR) return OfflineDuration.Minutes(minutes)
        val hours = minutes / MINUTES_PER_HOUR
        if (hours < HOURS_SHOWN) {
            val rest = minutes % MINUTES_PER_HOUR
            return if (rest == 0L) OfflineDuration.Hours(hours) else OfflineDuration.HoursMinutes(hours, rest)
        }
        return OfflineDuration.Days(hours / HOURS_PER_DAY)
    }
}
