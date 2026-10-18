package org.soos.remote.ui

import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import org.soos.remote.R
import org.soos.remote.tour.TourState
import org.soos.remote.tour.TourStep

/** The resources of one step; [where] is `null` for the introduction. */
private class TourTexts(
    @StringRes val title: Int,
    @StringRes val text: Int,
    @StringRes val off: Int,
    @StringRes val where: Int?,
)

/** One exhaustive map from a step to its fixed `strings.xml` texts. */
private fun tourTexts(step: TourStep): TourTexts = when (step) {
    TourStep.INTRO -> TourTexts(
        R.string.tour_intro_title,
        R.string.tour_intro_text,
        R.string.tour_intro_note,
        null,
    )
    TourStep.PHONE_COMPANION -> TourTexts(
        R.string.tour_phone_companion_title,
        R.string.tour_phone_companion_text,
        R.string.tour_phone_companion_off,
        R.string.tour_phone_companion_where,
    )
    TourStep.BATTERY -> TourTexts(
        R.string.tour_battery_title,
        R.string.tour_battery_text,
        R.string.tour_battery_off,
        R.string.tour_battery_where,
    )
    TourStep.REMOTE_LOCK -> TourTexts(
        R.string.tour_remote_lock_title,
        R.string.tour_remote_lock_text,
        R.string.tour_remote_lock_off,
        R.string.tour_remote_lock_where,
    )
    TourStep.INTERNET_ACCESS -> TourTexts(
        R.string.tour_internet_access_title,
        R.string.tour_internet_access_text,
        R.string.tour_internet_access_off,
        R.string.tour_internet_access_where,
    )
    TourStep.FAILED_PASSWORDS -> TourTexts(
        R.string.tour_failed_passwords_title,
        R.string.tour_failed_passwords_text,
        R.string.tour_failed_passwords_off,
        R.string.tour_failed_passwords_where,
    )
    TourStep.PUSH_ALERTS -> TourTexts(
        R.string.tour_push_alerts_title,
        R.string.tour_push_alerts_text,
        R.string.tour_push_alerts_off,
        R.string.tour_push_alerts_where,
    )
    TourStep.CAMERA_VIEW -> TourTexts(
        R.string.tour_camera_view_title,
        R.string.tour_camera_view_text,
        R.string.tour_camera_view_off,
        R.string.tour_camera_view_where,
    )
    TourStep.OFFLINE_ALERTS -> TourTexts(
        R.string.tour_offline_alerts_title,
        R.string.tour_offline_alerts_text,
        R.string.tour_offline_alerts_off,
        R.string.tour_offline_alerts_where,
    )
    TourStep.PC_OFFLINE_ALERTS -> TourTexts(
        R.string.tour_pc_offline_alerts_title,
        R.string.tour_pc_offline_alerts_text,
        R.string.tour_pc_offline_alerts_off,
        R.string.tour_pc_offline_alerts_where,
    )
}

/**
 * The feature tour card (GitHub #427): the step counter, the title (a heading for screen
 * readers), the explanation, the "Off:" line (the note on the introduction), where to turn the
 * feature off, then Back, Next or Done, and Skip tour. Fixed texts and theme roles only; no
 * animation.
 */
@Composable
internal fun TourCard(state: TourState, onNext: () -> Unit, onBack: () -> Unit, onSkip: () -> Unit) {
    val texts = tourTexts(state.step)
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                stringResource(R.string.tour_step_counter, state.number, TourState.TOTAL),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                stringResource(texts.title),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.semantics { heading() },
            )
            Text(stringResource(texts.text), style = MaterialTheme.typography.bodyMedium)
            Text(
                stringResource(texts.off),
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Bold,
            )
            texts.where?.let {
                Text(
                    stringResource(it),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Row(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (!state.isFirst) {
                    OutlinedButton(onClick = onBack) { Text(stringResource(R.string.tour_back)) }
                }
                Button(onClick = onNext) {
                    Text(stringResource(if (state.isLast) R.string.tour_done else R.string.tour_next))
                }
                if (!state.isLast) {
                    TextButton(onClick = onSkip) { Text(stringResource(R.string.tour_skip)) }
                }
            }
        }
    }
}
