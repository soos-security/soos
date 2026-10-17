package org.soos.remote.heartbeat

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import org.soos.remote.MainActivity
import org.soos.remote.R

/**
 * Posts the heartbeat's local notifications: the fixed texts of [HeartbeatTexts] only, never
 * without the notification permission, and a tap opens the app.
 */
class HeartbeatNotifier(private val context: Context) {
    fun show(notice: Notice) {
        if (context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        val channel = NotificationChannel(
            HeartbeatTexts.CHANNEL_ID,
            HeartbeatTexts.CHANNEL_NAME,
            NotificationManager.IMPORTANCE_DEFAULT,
        )
        channel.description = HeartbeatTexts.CHANNEL_DESCRIPTION
        manager.createNotificationChannel(channel)
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val text = HeartbeatTexts.of(notice)
        val notification = Notification.Builder(context, HeartbeatTexts.CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_monochrome)
            .setContentTitle(text.title)
            .setContentText(text.body)
            .setCategory(Notification.CATEGORY_STATUS)
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        manager.notify(HeartbeatTexts.notificationId(notice), notification)
    }
}
