package org.soos.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.soos.remote.heartbeat.HeartbeatTexts
import org.soos.remote.heartbeat.Notice

class HeartbeatTextsTest {
    @Test
    fun textsAreTheFixedEnglishSentences() {
        assertEquals("PC went offline", HeartbeatTexts.of(Notice.PC_OFFLINE).title)
        assertEquals(
            "soos could not reach your PC in the last two checks.",
            HeartbeatTexts.of(Notice.PC_OFFLINE).body,
        )
        assertEquals("PC back online", HeartbeatTexts.of(Notice.PC_BACK_ONLINE).title)
        assertEquals("soos can reach your PC again.", HeartbeatTexts.of(Notice.PC_BACK_ONLINE).body)
        assertEquals("Sign-in needed", HeartbeatTexts.of(Notice.SIGN_IN_NEEDED).title)
        assertEquals(
            "Open soos and sign in with your passkey to see the PC status.",
            HeartbeatTexts.of(Notice.SIGN_IN_NEEDED).body,
        )
    }

    @Test
    fun textsCarryNoPcData() {
        for (notice in Notice.entries) {
            val text = HeartbeatTexts.of(notice)
            for (part in listOf(text.title, text.body)) {
                assertTrue(part.isNotBlank())
                for (forbidden in listOf("ts.net", "https", "@", "$", "%", "{", "<")) {
                    assertFalse("$notice: $part", part.contains(forbidden))
                }
                assertTrue("$notice: ASCII only", part.all { it.code in 0x20..0x7e })
            }
        }
    }

    @Test
    fun backOnlineReplacesTheOfflineNotification() {
        assertEquals(
            HeartbeatTexts.notificationId(Notice.PC_OFFLINE),
            HeartbeatTexts.notificationId(Notice.PC_BACK_ONLINE),
        )
        assertNotEquals(
            HeartbeatTexts.notificationId(Notice.PC_OFFLINE),
            HeartbeatTexts.notificationId(Notice.SIGN_IN_NEEDED),
        )
        assertEquals("pc_status", HeartbeatTexts.CHANNEL_ID)
        assertEquals("PC status", HeartbeatTexts.CHANNEL_NAME)
    }
}
