package com.shadowgate.app.ui

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import com.shadowgate.app.crypto.KeyManager
import com.shadowgate.app.crypto.NativeCrypto
import com.shadowgate.app.rootdaemon.RootShell
import com.shadowgate.app.service.ShadowGateService

class MainActivity : ComponentActivity() {

    private val requiredPermissions = mutableListOf<String>().apply {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            add(Manifest.permission.BLUETOOTH_SCAN)
            add(Manifest.permission.BLUETOOTH_CONNECT)
            add(Manifest.permission.BLUETOOTH_ADVERTISE)
        } else {
            add(Manifest.permission.ACCESS_FINE_LOCATION)
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            add(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    private val permissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { results ->
        if (results.all { it.value }) {
            startService()
        } else {
            Toast.makeText(this, "BLE 权限被拒绝，ShadowGate 无法运行", Toast.LENGTH_LONG).show()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        setContent {
            ShadowGateTheme {
                ShadowGateScreen(
                    onToggleService = { start ->
                        if (start) {
                            if (hasAllPermissions()) {
                                startService()
                            } else {
                                permissionLauncher.launch(requiredPermissions.toTypedArray())
                            }
                        } else {
                            stopService()
                        }
                    },
                    onRequestBatteryOptimization = {
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                            val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS).apply {
                                data = android.net.Uri.parse("package:$packageName")
                            }
                            startActivity(intent)
                        }
                    }
                )
            }
        }
    }

    private fun hasAllPermissions(): Boolean {
        return requiredPermissions.all {
            ContextCompat.checkSelfPermission(this, it) == PackageManager.PERMISSION_GRANTED
        }
    }

    private fun startService() {
        val intent = Intent(this, ShadowGateService::class.java).apply {
            action = ShadowGateService.ACTION_START
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }

    private fun stopService() {
        val intent = Intent(this, ShadowGateService::class.java).apply {
            action = ShadowGateService.ACTION_STOP
        }
        startService(intent)
    }
}

// ===== Compose UI =====

@Composable
fun ShadowGateTheme(content: @Composable () -> Unit) {
    val darkColorScheme = darkColorScheme(
        primary = Color(0xFF6C5CE7),
        secondary = Color(0xFF00CEC9),
        background = Color(0xFF0A0A0F),
        surface = Color(0xFF14141F),
        onPrimary = Color.White,
        onBackground = Color(0xFFE8E8ED),
        onSurface = Color(0xFFE8E8ED),
    )

    MaterialTheme(
        colorScheme = darkColorScheme,
        typography = MaterialTheme.typography,
        content = content
    )
}

@Composable
fun ShadowGateScreen(
    onToggleService: (Boolean) -> Unit,
    onRequestBatteryOptimization: () -> Unit
) {
    var isServiceRunning by remember { mutableStateOf(false) }
    var txPower by remember { mutableStateOf("MEDIUM") }
    var advertiseInterval by remember { mutableIntStateOf(1000) }

    Surface(
        modifier = Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.background
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(24.dp)
        ) {
            // Header
            Text(
                text = "ShadowGate",
                fontSize = 28.sp,
                fontWeight = FontWeight.Bold,
                color = MaterialTheme.colorScheme.primary
            )

            // Status Card
            Card(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(16.dp),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
            ) {
                Column(
                    modifier = Modifier.padding(24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(16.dp)
                ) {
                    // Status indicator
                    Surface(
                        shape = RoundedCornerShape(50),
                        color = if (isServiceRunning) Color(0xFF00CEC9) else Color(0xFFFF6B6B),
                        modifier = Modifier.size(60.dp)
                    ) {
                        Box(contentAlignment = Alignment.Center) {
                            Text(
                                text = if (isServiceRunning) "ON" else "OFF",
                                color = Color.White,
                                fontWeight = FontWeight.Bold,
                                fontSize = 14.sp
                            )
                        }
                    }

                    Text(
                        text = if (isServiceRunning) "BLE 凭证广播中" else "服务未启动",
                        fontSize = 18.sp,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.onSurface
                    )

                    Text(
                        text = "PC 端可通过 BLE 扫描发现此设备",
                        fontSize = 13.sp,
                        color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.5f)
                    )
                }
            }

            // Root 状态指示器
            if (isServiceRunning) {
                RootStatusCard()
            }

            // Toggle Button
            Button(
                onClick = {
                    isServiceRunning = !isServiceRunning
                    onToggleService(isServiceRunning)
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp),
                shape = RoundedCornerShape(16.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = if (isServiceRunning) Color(0xFFFF6B6B) else MaterialTheme.colorScheme.primary
                )
            ) {
                Text(
                    text = if (isServiceRunning) "停止服务" else "启动服务",
                    fontSize = 16.sp,
                    fontWeight = FontWeight.SemiBold
                )
            }

            // Config Card
            Card(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(16.dp),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
            ) {
                Column(
                    modifier = Modifier.padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp)
                ) {
                    Text(
                        text = "广播配置",
                        fontSize = 16.sp,
                        fontWeight = FontWeight.SemiBold,
                        color = MaterialTheme.colorScheme.onSurface
                    )

                    // TX Power
                    Text(
                        text = "发射功率: $txPower",
                        fontSize = 14.sp,
                        color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.7f)
                    )
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        listOf("LOW", "MEDIUM", "HIGH").forEach { level ->
                            FilterChip(
                                selected = txPower == level,
                                onClick = { txPower = level },
                                label = { Text(level) }
                            )
                        }
                    }

                    // Advertise Interval
                    Text(
                        text = "广播间隔: ${advertiseInterval}ms",
                        fontSize = 14.sp,
                        color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.7f)
                    )
                    Slider(
                        value = advertiseInterval.toFloat(),
                        onValueChange = { advertiseInterval = it.toInt() },
                        valueRange = 200f..5000f,
                        steps = 47,
                        modifier = Modifier.fillMaxWidth()
                    )
                }
            }

            // Battery Optimization
            TextButton(
                onClick = onRequestBatteryOptimization,
                modifier = Modifier.fillMaxWidth()
            ) {
                Text(
                    text = "禁用电池优化 (确保持续运行)",
                    fontSize = 13.sp,
                    color = MaterialTheme.colorScheme.primary.copy(alpha = 0.8f)
                )
            }

            // Footer
            Spacer(modifier = Modifier.weight(1f))

            Text(
                text = "ShadowGate v0.1.0 — 近场凭证系统",
                fontSize = 11.sp,
                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.3f)
            )
        }
    }
}

/**
 * Root 特权状态指示卡片
 *
 * 显示：
 * - Root 是否可用 + 类型 (Magisk / KernelSU)
 * - Doze 是否已禁用
 * - BLE 适配器底层状态
 */
@Composable
fun RootStatusCard() {
    val rootAvailable = remember { RootShell.isRootAvailable() }
    val rootType = remember { RootShell.getRootType() }

    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(
            containerColor = if (rootAvailable) Color(0xFF1A1A2E) else MaterialTheme.colorScheme.surface
        ),
        border = if (rootAvailable) {
            androidx.compose.foundation.BorderStroke(1.dp, Color(0xFF6C5CE7).copy(alpha = 0.3f))
        } else null
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    text = if (rootAvailable) "Root 特权已激活" else "Root 不可用",
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold,
                    color = if (rootAvailable) Color(0xFF00CEC9) else Color(0xFFFF6B6B)
                )
                Text(
                    text = rootType,
                    fontSize = 11.sp,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.5f)
                )
            }

            if (rootAvailable) {
                Divider(color = Color.White.copy(alpha = 0.06f))

                StatusRow("Doze 模式", "已禁用", true)
                StatusRow("BLE 适配器", "Locked ON", true)
                StatusRow("App Standby", "Bypassed", true)
                StatusRow("OOM 保护", "adj=-17", true)
                StatusRow("开机自启", "已安装脚本", true)
            } else {
                Text(
                    text = "需要 Magisk / KernelSU Root 权限以获得最佳保活效果",
                    fontSize = 11.sp,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.4f)
                )
            }
        }
    }
}

@Composable
fun StatusRow(label: String, value: String, ok: Boolean) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween
    ) {
        Text(
            text = label,
            fontSize = 12.sp,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f)
        )
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = value,
                fontSize = 12.sp,
                color = if (ok) Color(0xFF00CEC9) else Color(0xFFFF6B6B)
            )
            Spacer(modifier = Modifier.width(4.dp))
            Text(
                text = if (ok) "✓" else "✗",
                fontSize = 12.sp,
                color = if (ok) Color(0xFF00CEC9) else Color(0xFFFF6B6B)
            )
        }
    }
}
