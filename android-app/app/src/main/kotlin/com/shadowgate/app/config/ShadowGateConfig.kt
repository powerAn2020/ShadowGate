package com.shadowgate.app.config

import java.util.UUID

object ShadowGateConfig {
    val SERVICE_UUID: UUID = UUID.fromString("7f4d0001-7d6a-4f8f-9a7d-4f1f68b0f001")
    val CHAR_CHALLENGE_UUID: UUID = UUID.fromString("7f4d0002-7d6a-4f8f-9a7d-4f1f68b0f001")
    val CHAR_RESPONSE_UUID: UUID = UUID.fromString("7f4d0003-7d6a-4f8f-9a7d-4f1f68b0f001")
    val CHAR_DEVICE_ID_UUID: UUID = UUID.fromString("7f4d0004-7d6a-4f8f-9a7d-4f1f68b0f001")

    const val DEFAULT_ADVERTISE_INTERVAL_MS = 1000
    const val DEFAULT_TX_POWER = "MEDIUM"
}
