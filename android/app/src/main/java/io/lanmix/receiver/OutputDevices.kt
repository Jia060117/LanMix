package io.lanmix.receiver

import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.os.Build

object OutputDevices {
    fun label(device: AudioDeviceInfo): String = "${device.productName} · ${type(device.type)} · #${device.id}"

    fun type(type: Int): String = when (type) {
        AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "Speaker"
        AudioDeviceInfo.TYPE_BUILTIN_EARPIECE -> "Earpiece"
        AudioDeviceInfo.TYPE_WIRED_HEADPHONES -> "Headphones"
        AudioDeviceInfo.TYPE_WIRED_HEADSET -> "Headset"
        AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_ACCESSORY,
        AudioDeviceInfo.TYPE_USB_HEADSET -> "USB"
        AudioDeviceInfo.TYPE_BLUETOOTH_A2DP -> "Bluetooth A2DP"
        AudioDeviceInfo.TYPE_BLE_HEADSET -> "Bluetooth LE"
        AudioDeviceInfo.TYPE_BLE_SPEAKER -> "Bluetooth LE speaker"
        AudioDeviceInfo.TYPE_HDMI, AudioDeviceInfo.TYPE_HDMI_ARC,
        AudioDeviceInfo.TYPE_HDMI_EARC -> "HDMI"
        AudioDeviceInfo.TYPE_LINE_ANALOG -> "Analog line"
        AudioDeviceInfo.TYPE_LINE_DIGITAL -> "Digital line"
        else -> "Type $type"
    }

    fun encoding(value: Int): String = when (value) {
        AudioFormat.ENCODING_PCM_FLOAT -> "PCM float"
        AudioFormat.ENCODING_PCM_16BIT -> "PCM 16-bit"
        AudioFormat.ENCODING_PCM_24BIT_PACKED -> "PCM 24-bit"
        AudioFormat.ENCODING_PCM_32BIT -> "PCM 32-bit"
        AudioFormat.ENCODING_PCM_8BIT -> "PCM 8-bit"
        else -> "Encoding $value"
    }

    fun report(device: AudioDeviceInfo): String = buildString {
        appendLine(label(device))
        appendLine("Rates: ${values(device.sampleRates)} Hz")
        appendLine("Channel counts: ${values(device.channelCounts)}")
        appendLine("Channel masks: ${masks(device.channelMasks)}")
        append("Encodings: ${device.encodings.joinToString { encoding(it) }.ifBlank { "Unspecified" }}")
        if (Build.VERSION.SDK_INT >= 31) {
            appendLine()
            appendLine("Reported profiles (${device.audioProfiles.size}):")
            if (device.audioProfiles.isEmpty()) append("  No profiles supplied by Android")
            device.audioProfiles.forEach { profile ->
                appendLine("  ${encoding(profile.format)} · ${values(profile.sampleRates)} Hz")
                appendLine("  masks ${masks(profile.channelMasks)}; index masks ${masks(profile.channelIndexMasks)}")
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

    private fun values(values: IntArray) = values.joinToString().ifBlank { "Unspecified" }
    private fun masks(values: IntArray) = values.joinToString { "0x${it.toString(16)}" }.ifBlank { "Unspecified" }
}
