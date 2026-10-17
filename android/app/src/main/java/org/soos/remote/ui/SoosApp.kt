package org.soos.remote.ui

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.soos.remote.R
import org.soos.remote.auth.PasskeyJson
import org.soos.remote.auth.PasskeyOutcome
import org.soos.remote.auth.PasskeySignIn
import org.soos.remote.config.PcUrl
import org.soos.remote.config.PcUrlError
import org.soos.remote.config.PcUrlResult
import org.soos.remote.data.SettingsStore
import org.soos.remote.heartbeat.HeartbeatController
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

    suspend fun refresh() {
        busy = true
        val authResult = withContext(Dispatchers.IO) { api.authState() }
        auth = (authResult as? ApiResult.Ok)?.value
        if (authResult is ApiResult.Unreachable) {
            status = null
            message = "The PC is unreachable. Check Tailscale or Funnel."
            busy = false
            return
        }
        val current = auth
        if (current != null && current.authenticated) {
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
        } else {
            status = null
        }
        busy = false
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

    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
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
