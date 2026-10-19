package tv.lounge.collector.inputs

import android.content.Context
import android.media.tv.TvInputInfo
import android.media.tv.TvInputManager
import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import tv.lounge.core.InputRecord
import tv.lounge.core.InputState

/**
 * Reads HDMI, AV and tuner inputs from [TvInputManager] with their connection state and CEC names.
 * Empty on a streamer. Which inputs show, and in what order, is the core's decision.
 */
class InputsCollector(private val context: Context) {
    private val manager: TvInputManager? = context.getSystemService(TvInputManager::class.java)

    fun snapshot(): List<InputRecord> {
        val tim = manager ?: return emptyList()
        return tim.tvInputList.orEmpty()
            .filter { it.isPassthroughInput || (it.type == TvInputInfo.TYPE_TUNER && it.id.contains("/HW")) }
            .map { info ->
                val state = runCatching { tim.getInputState(info.id) }.getOrDefault(TvInputManager.INPUT_STATE_DISCONNECTED)
                val label = info.loadLabel(context)?.toString()?.ifBlank { null } ?: fallbackLabel(info)
                InputRecord(
                    id = info.id,
                    label = label,
                    customLabel = info.loadCustomLabel(context)?.toString()?.ifBlank { null },
                    kind = when (info.type) {
                        TvInputInfo.TYPE_HDMI -> "hdmi"
                        TvInputInfo.TYPE_TUNER -> "tuner"
                        TvInputInfo.TYPE_COMPOSITE, TvInputInfo.TYPE_COMPONENT, TvInputInfo.TYPE_SVIDEO, TvInputInfo.TYPE_SCART,
                        TvInputInfo.TYPE_VGA, TvInputInfo.TYPE_DVI, TvInputInfo.TYPE_DISPLAY_PORT,
                        -> "av"
                        else -> "other"
                    },
                    hdmiPort = hdmiPort(label, info.id),
                    state = when (state) {
                        TvInputManager.INPUT_STATE_CONNECTED -> InputState.CONNECTED
                        TvInputManager.INPUT_STATE_CONNECTED_STANDBY -> InputState.STANDBY
                        else -> InputState.DISCONNECTED
                    },
                    `package` = info.serviceInfo.packageName,
                    passthrough = info.isPassthroughInput,
                )
            }
    }

    /** Emits when an input is added, removed, renamed or changes connection state. */
    fun changes(): Flow<Unit> = callbackFlow {
        val tim = manager
        if (tim == null) {
            awaitClose { }
            return@callbackFlow
        }
        val callback = object : TvInputManager.TvInputCallback() {
            override fun onInputStateChanged(inputId: String, state: Int) { trySend(Unit) }
            override fun onInputAdded(inputId: String) { trySend(Unit) }
            override fun onInputRemoved(inputId: String) { trySend(Unit) }
            override fun onInputUpdated(inputId: String) { trySend(Unit) }
        }
        tim.registerCallback(callback, Handler(Looper.getMainLooper()))
        awaitClose { runCatching { tim.unregisterCallback(callback) } }
    }

    private fun fallbackLabel(info: TvInputInfo): String = when (info.type) {
        TvInputInfo.TYPE_HDMI -> "HDMI"
        TvInputInfo.TYPE_TUNER -> "Live TV"
        else -> "Input"
    }

    /** The port number as the TV labels it ("HDMI 2"); the framework's port field is not public. */
    private fun hdmiPort(label: String, id: String): Int? =
        portInLabel.find(label)?.groupValues?.get(1)?.toIntOrNull()
            ?: portInId.find(id)?.groupValues?.get(1)?.toIntOrNull()

    private companion object {
        val portInLabel = Regex("HDMI\\s*(\\d)", RegexOption.IGNORE_CASE)
        val portInId = Regex("HDMI(\\d)")
    }
}
