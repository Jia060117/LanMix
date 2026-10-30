package io.lanmix.receiver

import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.os.Build

object OutputDevices {
    fun label(device: AudioDeviceInfo): String = "${device.productName} · ${type(device.type)} · #${device.id}"

    fun type(type: Int): String = when (type) {
        AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "扬声器"
        AudioDeviceInfo.TYPE_BUILTIN_EARPIECE -> "听筒"
        AudioDeviceInfo.TYPE_WIRED_HEADPHONES -> "有线耳机"
        AudioDeviceInfo.TYPE_WIRED_HEADSET -> "有线耳麦"
        AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_ACCESSORY,
        AudioDeviceInfo.TYPE_USB_HEADSET -> "USB"
        AudioDeviceInfo.TYPE_BLUETOOTH_A2DP -> "蓝牙 A2DP"
        AudioDeviceInfo.TYPE_BLE_HEADSET -> "蓝牙 LE"
        AudioDeviceInfo.TYPE_BLE_SPEAKER -> "蓝牙 LE 音箱"
        AudioDeviceInfo.TYPE_HDMI, AudioDeviceInfo.TYPE_HDMI_ARC,
        AudioDeviceInfo.TYPE_HDMI_EARC -> "HDMI"
        AudioDeviceInfo.TYPE_LINE_ANALOG -> "模拟线路"
        AudioDeviceInfo.TYPE_LINE_DIGITAL -> "数字线路"
        else -> "类型 $type"
    }

    fun encoding(value: Int): String = when (value) {
        AudioFormat.ENCODING_PCM_FLOAT -> "PCM float"
        AudioFormat.ENCODING_PCM_16BIT -> "PCM 16-bit"
        AudioFormat.ENCODING_PCM_24BIT_PACKED -> "PCM 24-bit"
        AudioFormat.ENCODING_PCM_32BIT -> "PCM 32-bit"
        AudioFormat.ENCODING_PCM_8BIT -> "PCM 8-bit"
        else -> "编码 $value"
    }

    fun report(device: AudioDeviceInfo): String = buildString {
        appendLine(label(device))
        appendLine("采样率：${values(device.sampleRates)} Hz")
        appendLine("声道数：${values(device.channelCounts)}")
        appendLine("声道掩码：${masks(device.channelMasks)}")
        append("编码：${device.encodings.joinToString { encoding(it) }.ifBlank { "未报告" }}")
        if (Build.VERSION.SDK_INT >= 31) {
            appendLine()
            appendLine("格式配置（${device.audioProfiles.size}）：")
            if (device.audioProfiles.isEmpty()) append("  系统未提供格式配置")
            device.audioProfiles.forEach { profile ->
                appendLine("  ${encoding(profile.format)} · ${values(profile.sampleRates)} Hz")
                appendLine("  声道掩码 ${masks(profile.channelMasks)}；索引掩码 ${masks(profile.channelIndexMasks)}")
            }
        }
    }.trimEnd()

    fun autoRate(manager: AudioManager, device: AudioDeviceInfo?): Int {
        val native = manager.getProperty(AudioManager.PROPERTY_OUTPUT_SAMPLE_RATE)?.toIntOrNull() ?: 48000
        val rates = device?.sampleRates?.filter { it in 8000..192000 }.orEmpty()
        return when {
            rates.isEmpty() || native in rates -> native.coerceIn(8000, 192000)
            48000 in rates -> 48000
            else -> rates.minBy { kotlin.math.abs(it - native) }
        }
    }

    private fun values(values: IntArray) = values.joinToString().ifBlank { "未报告" }
    private fun masks(values: IntArray) = values.joinToString { "0x${it.toString(16)}" }.ifBlank { "未报告" }
}
