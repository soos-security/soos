package org.soos.remote.ui

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.StringRes
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.colorResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import java.time.DateTimeException
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.soos.remote.R
import org.soos.remote.alerts.AckFeedback
import org.soos.remote.alerts.AlertsFeedback
import org.soos.remote.alerts.RefreshOutcome
import org.soos.remote.auth.PasskeyJson
import org.soos.remote.auth.PasskeyOutcome
import org.soos.remote.auth.PasskeySignIn
import org.soos.remote.config.PcUrl
import org.soos.remote.config.PcUrlError
import org.soos.remote.config.PcUrlResult
import org.soos.remote.data.SettingsStore
import org.soos.remote.heartbeat.HeartbeatController
import org.soos.remote.net.AlertRecord
import org.soos.remote.net.AlertsView
import org.soos.remote.net.ApiResult
import org.soos.remote.net.AuthStateView
import org.soos.remote.net.SharedApi
import org.soos.remote.net.StatusView

/** Root of the app: the settings screen until a PC address is saved, then the home screen. */
@Composable
fun SoosApp(activity: Activity, settings: SettingsStore, heartbeat: HeartbeatController) {
    var pcUrl by remember { mutableStateOf(settings.pcUrl) }
    var editing by remember { mutableStateOf(false) }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background),
    ) {
        Header()
        Box(
            modifier = Modifier
                .fillMaxSize()
                .navigationBarsPadding()
                .padding(16.dp),
        ) {
            val current = pcUrl
            val valid = current?.let { PcUrl.parse(it) as? PcUrlResult.Valid }
            if (valid == null || editing) {
                SettingsScreen(
                    initial = current.orEmpty(),
                    onSave = { result ->
                        // Saved through the heartbeat, which starts over for another PC.
                        heartbeat.savePcUrl(result.origin)
                        pcUrl = result.origin
                        editing = false
                    },
                    onCancel = if (valid != null) ({ editing = false }) else null,
                    activity = activity,
                    heartbeat = if (valid != null) heartbeat else null,
                )
            } else {
                HomeScreen(
                    activity = activity,
                    settings = settings,
                    pc = valid,
                    onSettings = { editing = true },
                )
            }
        }
    }
}

@Composable
private fun Header() {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .background(colorResource(R.color.soos_blue))
            .statusBarsPadding()
            .padding(horizontal = 16.dp, vertical = 12.dp),
    ) {
        Text(
            text = "soos",
            color = colorResource(R.color.soos_white),
            style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.Bold,
        )
    }
}

/** Fixed English explanation of a refused address (the input is never echoed). */
private fun urlErrorText(error: PcUrlError): String = when (error) {
    PcUrlError.EMPTY -> "Enter the address of your PC."
    PcUrlError.TOO_LONG -> "This address is too long."
    PcUrlError.NOT_HTTPS -> "The address must start with https://."
    PcUrlError.USERINFO -> "The address must not contain a user name."
    PcUrlError.PORT -> "The address must not contain a port."
    PcUrlError.PATH -> "Enter only the address, without a path."
    PcUrlError.QUERY_OR_FRAGMENT -> "Enter only the address, without ? or #."
    PcUrlError.INVALID_HOST -> "This is not a valid host name."
    PcUrlError.NOT_TS_NET -> "Use your PC's Tailscale name: https://<pc>.<tailnet>.ts.net."
}

@Composable
private fun SettingsScreen(
    initial: String,
    onSave: (PcUrlResult.Valid) -> Unit,
    onCancel: (() -> Unit)?,
    activity: Activity,
    heartbeat: HeartbeatController?,
) {
    var text by remember { mutableStateOf(initial) }
    var error by remember { mutableStateOf<String?>(null) }
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("PC address", style = MaterialTheme.typography.titleMedium)
        Text(
            "The address served by soos-remote through Tailscale, for example " +
                "https://my-pc.tailnet-name.ts.net.",
            style = MaterialTheme.typography.bodyMedium,
        )
        OutlinedTextField(
            value = text,
            onValueChange = {
                text = it.take(PcUrl.MAX_INPUT_LENGTH)
                error = null
            },
            singleLine = true,
            label = { Text("https://<pc>.<tailnet>.ts.net") },
            isError = error != null,
            modifier = Modifier.fillMaxWidth(),
        )
        error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = {
                when (val result = PcUrl.parse(text)) {
                    is PcUrlResult.Valid -> onSave(result)
                    is PcUrlResult.Invalid -> error = urlErrorText(result.reason)
                }
            }) { Text("Save") }
            if (onCancel != null) {
                OutlinedButton(onClick = onCancel) { Text("Cancel") }
            }
        }
        // Shown only once a valid PC address is saved.
        if (heartbeat != null) {
            HeartbeatCard(activity = activity, heartbeat = heartbeat)
        }
    }
}

/**
 * The opt-in PC-offline heartbeat (off by default). The notification permission is requested
 * here only, when the user turns the heartbeat on.
 */
@Composable
private fun HeartbeatCard(activity: Activity, heartbeat: HeartbeatController) {
    fun notificationsAllowed(): Boolean =
        activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED

    val blocked = "Notifications are blocked. Allow them for soos in the Android settings " +
        "to receive the PC alerts."
    var enabled by remember { mutableStateOf(heartbeat.enabled) }
    val refused = "The PC check could not be scheduled. Try again."
    var note by remember {
        mutableStateOf(
            when {
                heartbeat.scheduleRefused -> refused
                heartbeat.enabled && !notificationsAllowed() -> blocked
                else -> null
            },
        )
    }

    fun turnOn() {
        enabled = heartbeat.enable()
        note = if (enabled) null else refused
    }

    val contract = ActivityResultContracts.RequestPermission()
    val prompt = rememberLauncherForActivityResult(contract) { granted ->
        if (granted) {
            turnOn()
        } else {
            enabled = false
            note = blocked
        }
    }
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    "PC offline alerts",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                Switch(
                    checked = enabled,
                    onCheckedChange = { on ->
                        when {
                            !on -> {
                                heartbeat.disable()
                                enabled = false
                                note = null
                            }
                            notificationsAllowed() -> turnOn()
                            else -> prompt.launch(Manifest.permission.POST_NOTIFICATIONS)
                        }
                    },
                )
            }
            Text(
                "Checks your PC every 15 minutes in the background. You get one alert after " +
                    "two failed checks in a row, and one more when the PC is back online.",
                style = MaterialTheme.typography.bodyMedium,
            )
            note?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        }
    }
}

/** Fixed English text of a status. */
private fun stateText(state: String): String = when (state) {
    "locked" -> "Locked"
    "unlocked" -> "Unlocked"
    "no_session" -> "No local session"
    else -> "Status unavailable"
}

/** Fixed English text of a refusal (never the raw server body). */
private fun refusalText(status: Int, result: String?): String = when (result) {
    "already_locked" -> "The PC is already locked."
    "no_session" -> "No local session to lock."
    "rate_limited" -> "Too many attempts. Wait a moment and retry."
    "login_required" -> "Sign in with your passkey first."
    "passkey_rejected" -> "The passkey was not accepted."
    "no_passkey" -> "No passkey is registered on the PC yet."
    "passkeys_not_configured" -> "Passkeys are not configured on the PC."
    "forbidden" -> "The PC refused the request."
    "busy", "unavailable" -> "The PC is busy. Retry in a moment."
    else -> "The PC refused the request ($status)."
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HomeScreen(
    activity: Activity,
    settings: SettingsStore,
    pc: PcUrlResult.Valid,
    onSettings: () -> Unit,
) {
    // One client per PC for the whole process: the heartbeat reuses its in-memory session.
    val api = remember(pc.origin) { SharedApi.forOrigin(pc.origin) }
    val scope = rememberCoroutineScope()
    var auth by remember { mutableStateOf<AuthStateView?>(null) }
    var status by remember { mutableStateOf<StatusView?>(null) }
    var message by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    var alerts by remember { mutableStateOf<AlertsView?>(null) }
    var alertsFailed by remember { mutableStateOf(false) }
    var ackFeedback by remember { mutableStateOf<AckFeedback?>(null) }

    fun clearAlerts() {
        alerts = null
        alertsFailed = false
        ackFeedback = null
    }

    suspend fun loadAlerts() {
        when (val result = withContext(Dispatchers.IO) { api.alerts() }) {
            is ApiResult.Ok -> {
                alerts = result.value
                alertsFailed = false
            }
            is ApiResult.Refused, ApiResult.Unreachable -> {
                alerts = null
                alertsFailed = true
            }
        }
    }

    /**
     * Reloads the auth state, the status and the alerts. Every refresh drops the previous
     * acknowledgement feedback first, so it never outlives the view it was about; [carried] is
     * the refusal of the acknowledgement that asked for this refresh, shown again under the
     * fresh view ([AlertsFeedback.afterRefresh]).
     */
    suspend fun refresh(carried: AckFeedback? = null) {
        ackFeedback = null
        busy = true
        val authResult = withContext(Dispatchers.IO) { api.authState() }
        auth = (authResult as? ApiResult.Ok)?.value
        val current = auth
        val outcome = if (authResult is ApiResult.Unreachable) {
            status = null
            clearAlerts()
            message = "The PC is unreachable. Check Tailscale or Funnel."
            RefreshOutcome.UNREACHABLE
        } else if (current != null && current.authenticated) {
            when (val result = withContext(Dispatchers.IO) { api.status() }) {
                is ApiResult.Ok -> {
                    status = result.value
                    message = null
                }
                is ApiResult.Refused -> {
                    status = null
                    message = refusalText(result.status, result.result)
                }
                ApiResult.Unreachable -> {
                    status = null
                    message = "The PC is unreachable. Check Tailscale or Funnel."
                }
            }
            loadAlerts()
            RefreshOutcome.AUTHENTICATED
        } else {
            status = null
            clearAlerts()
            RefreshOutcome.SIGNED_OUT
        }
        val shown = AlertsFeedback.afterRefresh(carried, outcome)
        ackFeedback = shown.card
        // Signed out, the alerts card is hidden: its login-required text becomes the message.
        shown.notice?.let { message = activity.getString(ackFeedbackText(it)) }
        busy = false
    }

    suspend fun acknowledge(view: AlertsView) {
        val epoch = view.epoch ?: return
        busy = true
        val result = withContext(Dispatchers.IO) { api.ackAlerts(epoch, view.through) }
        val feedback = AlertsFeedback.ofAck(result)
        if (result is ApiResult.Ok) {
            alerts = result.value
            alertsFailed = false
        }
        ackFeedback = feedback
        busy = false
        // A refused acknowledgement shows the fresh view (never acknowledged blindly) and keeps
        // its fixed text under it.
        if (AlertsFeedback.needsRefresh(result)) refresh(carried = feedback)
    }

    suspend fun signIn() {
        busy = true
        message = null
        val options = when (val result = withContext(Dispatchers.IO) { api.loginOptions(pc.host) }) {
            is ApiResult.Ok -> result.value
            is ApiResult.Refused -> {
                message = refusalText(result.status, result.result)
                busy = false
                return
            }
            ApiResult.Unreachable -> {
                message = "The PC is unreachable. Check Tailscale or Funnel."
                busy = false
                return
            }
        }
        val request = PasskeyJson.requestJson(options, settings.credentialId)
        when (val outcome = PasskeySignIn(activity).getAssertion(request)) {
            is PasskeyOutcome.Response -> {
                val body = PasskeyJson.assertionBody(outcome.json)
                if (body == null) {
                    message = "The passkey answer could not be read."
                } else {
                    when (val verified = withContext(Dispatchers.IO) { api.loginVerify(body.json) }) {
                        is ApiResult.Ok -> settings.credentialId = body.credentialId
                        is ApiResult.Refused ->
                            message = refusalText(verified.status, verified.result)
                        ApiResult.Unreachable ->
                            message = "The PC is unreachable. Check Tailscale or Funnel."
                    }
                }
            }
            PasskeyOutcome.Cancelled -> message = "Sign-in cancelled."
            PasskeyOutcome.NoCredential -> {
                // The remembered passkey is gone: the next attempt lets the provider choose.
                settings.credentialId = null
                message = "No passkey found for this PC. Try again to choose one."
            }
            PasskeyOutcome.Failed -> message = "The passkey prompt failed."
        }
        busy = false
        refresh()
    }

    LaunchedEffect(pc.origin) { refresh() }

    PullToRefreshBox(
        isRefreshing = refreshing,
        onRefresh = {
            if (!busy) {
                scope.launch {
                    refreshing = true
                    refresh()
                    refreshing = false
                }
            }
        },
        modifier = Modifier.fillMaxSize(),
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(pc.host, style = MaterialTheme.typography.bodySmall)
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp)) {
                    val current = auth
                    val text = when {
                        current == null -> "Connecting..."
                        !current.authenticated -> "Signed out"
                        status != null -> stateText(status?.state.orEmpty())
                        else -> "Status unavailable"
                    }
                    Text(text, style = MaterialTheme.typography.headlineSmall)
                    status?.let {
                        if (it.idle) Text("Idle", style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
            message?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            val current = auth
            if (current != null && current.mode == "funnel" && !current.authenticated) {
                Button(
                    enabled = !busy,
                    onClick = { scope.launch { signIn() } },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Sign in with passkey") }
            }
            if (current != null && current.authenticated) {
                Button(
                    enabled = !busy && status?.state == "unlocked",
                    onClick = {
                        scope.launch {
                            busy = true
                            when (val result = withContext(Dispatchers.IO) { api.lock() }) {
                                is ApiResult.Ok -> message = null
                                is ApiResult.Refused ->
                                    message = refusalText(result.status, result.result)
                                ApiResult.Unreachable ->
                                    message = "The PC is unreachable. Check Tailscale or Funnel."
                            }
                            busy = false
                            refresh()
                        }
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Lock PC") }
                AlertsCard(
                    view = alerts,
                    failed = alertsFailed,
                    feedback = ackFeedback,
                    busy = busy,
                    onAcknowledge = { view ->
                        ackFeedback = null
                        scope.launch { acknowledge(view) }
                    },
                )
            }
            Spacer(modifier = Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(enabled = !busy, onClick = { scope.launch { refresh() } }) {
                    Text("Refresh")
                }
                if (current != null && current.mode == "funnel" && current.authenticated) {
                    OutlinedButton(
                        enabled = !busy,
                        onClick = {
                            scope.launch {
                                withContext(Dispatchers.IO) { api.logout() }
                                refresh()
                            }
                        },
                    ) { Text("Sign out") }
                }
                TextButton(onClick = onSettings) { Text("Settings") }
            }
        }
    }
}

/** Fixed text of an acknowledgement feedback (a server token only selects the feedback). */
@StringRes
private fun ackFeedbackText(feedback: AckFeedback): Int = when (feedback) {
    AckFeedback.DONE -> R.string.alerts_ack_done
    AckFeedback.STALE_VIEW -> R.string.alerts_ack_stale_view
    AckFeedback.RATE_LIMITED -> R.string.alerts_ack_rate_limited
    AckFeedback.DISABLED -> R.string.alerts_ack_disabled
    AckFeedback.UNAVAILABLE -> R.string.alerts_ack_unavailable
    AckFeedback.BAD_REQUEST -> R.string.alerts_ack_bad_request
    AckFeedback.LOGIN_REQUIRED -> R.string.alerts_ack_login_required
    AckFeedback.REFUSED -> R.string.alerts_ack_refused
    AckFeedback.UNREACHABLE -> R.string.alerts_ack_unreachable
}

@StringRes
private fun alertsReasonText(reason: String?): Int? = when (reason) {
    "no_journal_access" -> R.string.alerts_reason_no_journal_access
    "journal_reader_failed" -> R.string.alerts_reason_journal_reader_failed
    "owner_unresolved" -> R.string.alerts_reason_owner_unresolved
    "overflow" -> R.string.alerts_reason_overflow
    "rng_failed" -> R.string.alerts_reason_rng_failed
    else -> null
}

@StringRes
private fun alertSourceText(source: String?): Int = when (source) {
    "lock_screen" -> R.string.alerts_source_lock_screen
    "sudo" -> R.string.alerts_source_sudo
    "login" -> R.string.alerts_source_login
    else -> R.string.alerts_source_other
}

@StringRes
private fun alertAccountText(account: String): Int = when (account) {
    "owner" -> R.string.alerts_account_owner
    "root" -> R.string.alerts_account_root
    else -> R.string.alerts_account_other
}

/** A journal time in the phone's time zone and locale, or `null` when it cannot be shown. */
private fun alertTime(unixMs: Long): String? =
    try {
        DateTimeFormatter.ofLocalizedDateTime(FormatStyle.SHORT)
            .withZone(ZoneId.systemDefault())
            .format(Instant.ofEpochMilli(unixMs))
    } catch (ignored: DateTimeException) {
        null
    }

@Composable
private fun timeText(unixMs: Long): String =
    alertTime(unixMs) ?: stringResource(R.string.alerts_time_unknown)

/** `count` with the `_one` text for exactly one, the `_other` text otherwise. */
@Composable
private fun countText(count: Long, @StringRes one: Int, @StringRes other: Int): String =
    stringResource(if (count == 1L) one else other, count)

@Composable
private fun recordText(record: AlertRecord): String {
    val what = if (record.kind == "locked_out") {
        countText(record.count, R.string.alerts_record_locked_out_one, R.string.alerts_record_locked_out_other)
    } else {
        countText(record.count, R.string.alerts_record_wrong_password_one, R.string.alerts_record_wrong_password_other)
    }
    return stringResource(
        R.string.alerts_history_row,
        timeText(record.lastUnixMs),
        stringResource(alertSourceText(record.source)),
        stringResource(alertAccountText(record.account)),
        what,
    )
}

/**
 * Failed-password alerts (`Docs/REMOTE_COMPANION.md` §2c, §2j): one fixed sentence per state,
 * the unacknowledged counts, the history in the phone's time zone and the Acknowledge button.
 */
@Composable
private fun AlertsCard(
    view: AlertsView?,
    failed: Boolean,
    feedback: AckFeedback?,
    busy: Boolean,
    onAcknowledge: (AlertsView) -> Unit,
) {
    if (view == null && !failed) return
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(stringResource(R.string.alerts_title), style = MaterialTheme.typography.titleMedium)
            if (view == null) {
                Text(stringResource(R.string.alerts_load_failed), color = MaterialTheme.colorScheme.error)
            } else {
                val wrong = view.unacknowledgedWrongPassword
                val lockedOut = view.unacknowledgedLockedOut
                val pending = view.state != "disabled" && (wrong > 0 || lockedOut > 0)
                val notMonitored = view.lockScreen == "not_configured"
                val summary: Int? = when (view.state) {
                    "disabled" -> R.string.alerts_state_disabled
                    "starting" -> R.string.alerts_state_starting
                    "unavailable" -> R.string.alerts_state_unavailable
                    else -> if (pending) {
                        null
                    } else if (notMonitored) {
                        R.string.alerts_state_active_none_partial
                    } else {
                        R.string.alerts_state_active_none
                    }
                }
                summary?.let { Text(stringResource(it)) }
                if (view.state == "unavailable") {
                    alertsReasonText(view.reason)?.let { Text(stringResource(it)) }
                }
                if (pending) {
                    if (wrong > 0) {
                        Text(
                            countText(wrong, R.string.alerts_wrong_password_count_one, R.string.alerts_wrong_password_count_other),
                            color = MaterialTheme.colorScheme.error,
                            fontWeight = FontWeight.Bold,
                        )
                    }
                    if (lockedOut > 0) {
                        Text(
                            countText(lockedOut, R.string.alerts_locked_out_count_one, R.string.alerts_locked_out_count_other),
                            color = MaterialTheme.colorScheme.error,
                            fontWeight = FontWeight.Bold,
                        )
                    }
                    view.lastUnixMs?.let { last ->
                        Text(
                            stringResource(
                                R.string.alerts_last_attempt,
                                timeText(last),
                                stringResource(alertSourceText(view.lastSource)),
                            ),
                        )
                    }
                }
                if (notMonitored && view.state != "disabled") {
                    Text(
                        stringResource(R.string.alerts_lock_screen_not_monitored),
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                if (view.history.isNotEmpty()) {
                    Text(stringResource(R.string.alerts_history_title), style = MaterialTheme.typography.titleSmall)
                    for (record in view.history) {
                        Text(recordText(record), style = MaterialTheme.typography.bodyMedium)
                    }
                }
                // Same rule as the web app (`crates/remote/assets/app.js`, `alertsAckButton`):
                // shown while counts are pending and the view has an epoch, whatever the state.
                // A `starting` or `unavailable` state with a book still acknowledges (200);
                // without a book the server view is empty (no epoch, zero counts), so the button
                // is already hidden, and `disabled` is excluded by `pending` (403 alerts_disabled).
                if (pending && view.epoch != null) {
                    Button(
                        enabled = !busy,
                        onClick = { onAcknowledge(view) },
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text(stringResource(R.string.alerts_acknowledge)) }
                }
            }
            feedback?.let {
                Text(stringResource(ackFeedbackText(it)), style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}
