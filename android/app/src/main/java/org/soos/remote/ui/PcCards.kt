package org.soos.remote.ui

import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import org.soos.remote.R
import org.soos.remote.cards.BatteryLine
import org.soos.remote.cards.BatteryPower
import org.soos.remote.cards.CardModel
import org.soos.remote.cards.OfflineDuration
import org.soos.remote.net.BatteryView
import org.soos.remote.net.OfflineInterval
import org.soos.remote.net.OfflineView

/*
 * Read-only battery and offline-history cards of the home screen (GitHub #439, part of #357;
 * `Docs/REMOTE_COMPANION.md` §2g, §2j). Every text is a fixed strings.xml resource: a server
 * value only selects one (the parsers of net/ApiJson.kt admit known values only). Colours come
 * from the theme. The cards make no request and have no action.
 */

/** The battery card: nothing before the first load, a fixed text when it could not be read. */
@Composable
fun BatteryCard(view: BatteryView?, failed: Boolean) {
    if (view == null && !failed) return
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            CardTitle(R.string.battery_title)
            if (view == null) {
                Text(stringResource(R.string.battery_load_failed), color = MaterialTheme.colorScheme.error)
            } else {
                val line = when (CardModel.batteryLine(view)) {
                    BatteryLine.DISABLED -> stringResource(R.string.battery_disabled)
                    BatteryLine.UNAVAILABLE -> stringResource(R.string.battery_unavailable)
                    BatteryLine.NO_BATTERY -> stringResource(R.string.battery_no_battery)
                    BatteryLine.LEVEL -> stringResource(R.string.battery_level, view.percent ?: 0)
                    BatteryLine.LEVEL_UNKNOWN -> stringResource(R.string.battery_level_unknown)
                }
                Text(line, style = MaterialTheme.typography.bodyLarge)
                CardModel.batteryPower(view)?.let { power ->
                    Text(stringResource(batteryPowerText(power)), style = MaterialTheme.typography.bodyMedium)
                }
            }
        }
    }
}

/** The offline-history card: the recorded periods, newest first, in the phone's time zone. */
@Composable
fun OfflineCard(view: OfflineView?, failed: Boolean) {
    if (view == null && !failed) return
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            CardTitle(R.string.offline_title)
            if (view == null) {
                Text(stringResource(R.string.offline_load_failed), color = MaterialTheme.colorScheme.error)
            } else {
                when (view.state) {
                    "disabled" -> Text(stringResource(R.string.offline_disabled))
                    // The only unavailable reason is `store_failed` (ApiJson.OFFLINE_UNAVAILABLE_REASONS).
                    "unavailable" -> Text(stringResource(R.string.offline_unavailable))
                    else -> OfflineHistory(view)
                }
            }
        }
    }
}

@Composable
private fun CardTitle(@StringRes title: Int) {
    Text(
        stringResource(title),
        style = MaterialTheme.typography.titleMedium,
        modifier = Modifier.semantics { heading() },
    )
}

/** The `active` view: the count, the monitor warning and one line per interval. */
@Composable
private fun OfflineHistory(view: OfflineView) {
    val count = view.intervals.size
    val summary = if (count == 0) {
        stringResource(R.string.offline_none)
    } else {
        stringResource(if (count == 1) R.string.offline_count_one else R.string.offline_count_other, count)
    }
    Text(summary)
    if (view.networkMonitor == "unavailable") {
        Text(stringResource(R.string.offline_not_monitored), style = MaterialTheme.typography.bodySmall)
    }
    for (interval in view.intervals) {
        Text(intervalText(interval), style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun intervalText(interval: OfflineInterval): String =
    stringResource(
        R.string.offline_row,
        timeText(interval.startUnixMs),
        timeText(interval.endUnixMs),
        durationText(CardModel.offlineDuration(interval.durationMs)),
        stringResource(offlineReasonText(interval.reason)),
    )

@Composable
private fun durationText(duration: OfflineDuration): String = when (duration) {
    OfflineDuration.UnderMinute -> stringResource(R.string.offline_duration_under_minute)
    is OfflineDuration.Minutes -> stringResource(R.string.offline_duration_minutes, duration.minutes)
    is OfflineDuration.Hours -> stringResource(R.string.offline_duration_hours, duration.hours)
    is OfflineDuration.HoursMinutes ->
        stringResource(R.string.offline_duration_hours_minutes, duration.hours, duration.minutes)
    is OfflineDuration.Days -> stringResource(R.string.offline_duration_days, duration.days)
}

@StringRes
private fun batteryPowerText(power: BatteryPower): Int = when (power) {
    BatteryPower.CHARGING -> R.string.battery_charging
    BatteryPower.ON_BATTERY -> R.string.battery_on_battery
    BatteryPower.FULL -> R.string.battery_full
    BatteryPower.PLUGGED_IN_NOT_CHARGING -> R.string.battery_plugged_in_not_charging
    BatteryPower.PLUGGED_IN -> R.string.battery_plugged_in
}

/** [ApiJson.OFFLINE_REASONS][org.soos.remote.net.ApiJson.OFFLINE_REASONS] only reach this map. */
@StringRes
private fun offlineReasonText(reason: String): Int = when (reason) {
    "network_lost" -> R.string.offline_reason_network_lost
    "wifi_disabled" -> R.string.offline_reason_wifi_disabled
    "sleep" -> R.string.offline_reason_sleep
    else -> R.string.offline_reason_shutdown_or_crash
}
