package org.soos.remote.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.colorResource
import org.soos.remote.R

/**
 * soos brand theme: every colour comes from `res/values/colors.xml`, whose values equal
 * `crates/gui/src/theme.rs` (no colour literal in Kotlin). Light is the brand default; the
 * dark variant uses the same palette.
 */
@Composable
fun SoosTheme(content: @Composable () -> Unit) {
    val blue = colorResource(R.color.soos_blue)
    val pale = colorResource(R.color.soos_pale)
    val ink = colorResource(R.color.soos_ink)
    val white = colorResource(R.color.soos_white)
    val danger = colorResource(R.color.soos_danger)
    val scheme = if (isSystemInDarkTheme()) {
        darkColorScheme(
            primary = blue,
            onPrimary = white,
            secondary = colorResource(R.color.soos_pink),
            onSecondary = ink,
            background = ink,
            onBackground = pale,
            surface = ink,
            onSurface = pale,
            surfaceVariant = ink,
            onSurfaceVariant = colorResource(R.color.soos_pale_4),
            outline = colorResource(R.color.soos_ink_muted),
            error = danger,
            onError = white,
        )
    } else {
        lightColorScheme(
            primary = blue,
            onPrimary = white,
            secondary = colorResource(R.color.soos_pink),
            onSecondary = ink,
            background = pale,
            onBackground = ink,
            surface = white,
            onSurface = ink,
            surfaceVariant = pale,
            onSurfaceVariant = colorResource(R.color.soos_ink_muted),
            outline = colorResource(R.color.soos_line),
            error = danger,
            onError = white,
        )
    }
    MaterialTheme(colorScheme = scheme, content = content)
}
